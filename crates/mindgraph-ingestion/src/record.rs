//! Streaming record readers for CSV and flat JSON.
//!
//! - **CSV**: UTF-8, a header row of unique non-empty names, equal-length
//!   records. Blank cells are null. Values stay untyped text ([`FieldValue::Raw`]).
//! - **JSON**: a top-level array of flat objects, read one object at a time.
//!   Values: null, boolean, number, string, array of strings. Nested objects
//!   and other arrays are rejected, never flattened.
//!
//! Both readers stream: memory use is bounded by one record, not the file.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{BufReader, Read};

use mindgraph_domain::DataSourceKind;
use serde::de::{self, DeserializeSeed, SeqAccess, Visitor};
use serde_json::Value;

use crate::IngestionError;

/// One value of a source record.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    /// Blank (CSV) or `null` (JSON).
    Null,
    /// Untyped CSV text; its type is inferred or converted later.
    Raw(String),
    /// A JSON string.
    Str(String),
    /// A JSON boolean.
    Bool(bool),
    /// A JSON integer.
    Integer(i64),
    /// A JSON non-integer number.
    Float(f64),
    /// A JSON array of strings.
    StringArray(Vec<String>),
}

/// One source record: field name → value. Fields absent from a JSON object
/// are simply missing.
pub(crate) type Record = BTreeMap<String, FieldValue>;

/// Resource bounds applied while reading a dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum number of records.
    pub max_records: u64,
    /// Maximum number of distinct fields.
    pub max_fields: usize,
    /// Maximum size of one value, in bytes.
    pub max_value_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_records: 100_000,
            max_fields: 500,
            max_value_bytes: 64 * 1024,
        }
    }
}

/// What a full read of a dataset found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordStats {
    pub(crate) record_count: u64,
    /// CSV header order, or JSON keys in sorted order.
    pub(crate) fields: Vec<String>,
}

pub(crate) type RecordCallback<'a> = dyn FnMut(u64, Record) -> Result<(), IngestionError> + 'a;

/// Reads every record, calling `on_record` with its 1-based number.
pub(crate) fn for_each_record(
    kind: DataSourceKind,
    reader: impl Read,
    limits: &Limits,
    on_record: &mut RecordCallback<'_>,
) -> Result<RecordStats, IngestionError> {
    match kind {
        DataSourceKind::Csv => read_csv(reader, limits, on_record),
        DataSourceKind::Json => read_json(reader, limits, on_record),
    }
}

fn csv_error(err: &csv::Error) -> IngestionError {
    let line = err.position().map(csv::Position::line);
    let at = line
        .map(|line| format!(" (line {line})"))
        .unwrap_or_default();
    match err.kind() {
        csv::ErrorKind::Utf8 { .. } => {
            IngestionError::MalformedDataset(format!("CSV is not valid UTF-8{at}"))
        }
        csv::ErrorKind::UnequalLengths {
            expected_len, len, ..
        } => IngestionError::MalformedDataset(format!(
            "CSV record has {len} fields but the header has {expected_len}{at}"
        )),
        csv::ErrorKind::Io(io) => IngestionError::SourceStore {
            message: format!("reading the source file failed: {io}"),
            source: None,
        },
        _ => IngestionError::MalformedDataset(format!("invalid CSV{at}")),
    }
}

fn check_value(
    limits: &Limits,
    record: u64,
    field: &str,
    len: usize,
) -> Result<(), IngestionError> {
    if len > limits.max_value_bytes {
        return Err(IngestionError::LimitExceeded(format!(
            "record {record}: field `{field}` is larger than {} bytes",
            limits.max_value_bytes
        )));
    }
    Ok(())
}

fn check_record_count(limits: &Limits, record: u64) -> Result<(), IngestionError> {
    if record > limits.max_records {
        return Err(IngestionError::LimitExceeded(format!(
            "the dataset has more than {} records",
            limits.max_records
        )));
    }
    Ok(())
}

fn read_csv(
    reader: impl Read,
    limits: &Limits,
    on_record: &mut RecordCallback<'_>,
) -> Result<RecordStats, IngestionError> {
    let mut csv = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .from_reader(BufReader::new(reader));
    let headers = csv.headers().map_err(|err| csv_error(&err))?.clone();
    if headers.is_empty() {
        return Err(IngestionError::InvalidSchema(
            "the CSV has no header row".to_owned(),
        ));
    }
    if headers.len() > limits.max_fields {
        return Err(IngestionError::LimitExceeded(format!(
            "the CSV has more than {} columns",
            limits.max_fields
        )));
    }
    let mut fields = Vec::with_capacity(headers.len());
    let mut seen = BTreeSet::new();
    for (index, header) in headers.iter().enumerate() {
        let name = header.trim();
        if name.is_empty() {
            return Err(IngestionError::InvalidSchema(format!(
                "column {} has an empty header",
                index + 1
            )));
        }
        if !seen.insert(name.to_owned()) {
            return Err(IngestionError::InvalidSchema(format!(
                "duplicate header `{name}`"
            )));
        }
        fields.push(name.to_owned());
    }

    let mut count = 0_u64;
    for row in csv.records() {
        let row = row.map_err(|err| csv_error(&err))?;
        count += 1;
        check_record_count(limits, count)?;
        let mut record = Record::new();
        for (name, value) in fields.iter().zip(row.iter()) {
            check_value(limits, count, name, value.len())?;
            let value = if value.trim().is_empty() {
                FieldValue::Null
            } else {
                FieldValue::Raw(value.to_owned())
            };
            record.insert(name.clone(), value);
        }
        on_record(count, record)?;
    }
    Ok(RecordStats {
        record_count: count,
        fields,
    })
}

/// State of the streaming JSON visitor.
struct JsonRecords<'a, 'b> {
    limits: &'a Limits,
    on_record: &'a mut RecordCallback<'b>,
    count: u64,
    fields: BTreeSet<String>,
    /// The first callback or validation error; serde only sees a marker.
    failure: Option<IngestionError>,
}

impl JsonRecords<'_, '_> {
    fn convert(
        &mut self,
        object: serde_json::Map<String, Value>,
    ) -> Result<Record, IngestionError> {
        let record_no = self.count;
        let mut record = Record::new();
        for (name, value) in object {
            let unsupported = |what: &str| {
                IngestionError::UnsupportedFormat(format!(
                    "record {record_no}: field `{name}` {what}"
                ))
            };
            let value = match value {
                Value::Null => FieldValue::Null,
                Value::Bool(flag) => FieldValue::Bool(flag),
                Value::Number(number) => match (number.as_i64(), number.as_f64()) {
                    (Some(integer), _) if !number.is_f64() => FieldValue::Integer(integer),
                    (_, Some(float)) if number.is_f64() => FieldValue::Float(float),
                    _ => return Err(unsupported("is a number outside the 64-bit range")),
                },
                Value::String(text) => {
                    check_value(self.limits, record_no, &name, text.len())?;
                    FieldValue::Str(text)
                }
                Value::Array(items) => {
                    let mut strings = Vec::with_capacity(items.len());
                    for item in items {
                        match item {
                            Value::String(text) => {
                                check_value(self.limits, record_no, &name, text.len())?;
                                strings.push(text);
                            }
                            _ => {
                                return Err(unsupported(
                                    "is an array; only arrays of strings are supported",
                                ));
                            }
                        }
                    }
                    FieldValue::StringArray(strings)
                }
                Value::Object(_) => {
                    return Err(unsupported(
                        "is a nested object; nested objects are not supported",
                    ));
                }
            };
            if !self.fields.contains(&name) {
                if self.fields.len() >= self.limits.max_fields {
                    return Err(IngestionError::LimitExceeded(format!(
                        "the dataset has more than {} fields",
                        self.limits.max_fields
                    )));
                }
                self.fields.insert(name.clone());
            }
            record.insert(name, value);
        }
        Ok(record)
    }
}

impl<'de> Visitor<'de> for &mut JsonRecords<'_, '_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a top-level JSON array of objects")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        loop {
            let next = self.count + 1;
            if let Err(err) = check_record_count(self.limits, next) {
                self.failure = Some(err);
                return Err(de::Error::custom("limit"));
            }
            let Some(element) = seq.next_element::<Value>()? else {
                return Ok(());
            };
            self.count = next;
            let Value::Object(object) = element else {
                self.failure = Some(IngestionError::MalformedDataset(format!(
                    "record {next} is not a JSON object"
                )));
                return Err(de::Error::custom("not an object"));
            };
            let result = self
                .convert(object)
                .and_then(|record| (self.on_record)(next, record));
            if let Err(err) = result {
                self.failure = Some(err);
                return Err(de::Error::custom("aborted"));
            }
        }
    }
}

impl<'de> DeserializeSeed<'de> for &mut JsonRecords<'_, '_> {
    type Value = ();

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

fn read_json(
    reader: impl Read,
    limits: &Limits,
    on_record: &mut RecordCallback<'_>,
) -> Result<RecordStats, IngestionError> {
    let mut state = JsonRecords {
        limits,
        on_record,
        count: 0,
        fields: BTreeSet::new(),
        failure: None,
    };
    let mut deserializer = serde_json::Deserializer::from_reader(BufReader::new(reader));
    let result = (&mut state)
        .deserialize(&mut deserializer)
        .and_then(|()| deserializer.end());
    if let Some(failure) = state.failure.take() {
        return Err(failure);
    }
    // Report position and category only: serde messages can quote data.
    result.map_err(|err| match err.classify() {
        serde_json::error::Category::Io => IngestionError::SourceStore {
            message: "reading the source file failed".to_owned(),
            source: Some(Box::new(err)),
        },
        category => IngestionError::MalformedDataset(format!(
            "expected a top-level JSON array of objects ({category:?} error at line {}, column {})",
            err.line(),
            err.column()
        )),
    })?;
    Ok(RecordStats {
        record_count: state.count,
        fields: state.fields.into_iter().collect(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn read(
        kind: DataSourceKind,
        data: &str,
    ) -> Result<(RecordStats, Vec<Record>), IngestionError> {
        read_with(kind, data, &Limits::default())
    }

    pub(crate) fn read_with(
        kind: DataSourceKind,
        data: &str,
        limits: &Limits,
    ) -> Result<(RecordStats, Vec<Record>), IngestionError> {
        let mut records = Vec::new();
        let stats = for_each_record(kind, data.as_bytes(), limits, &mut |_, record| {
            records.push(record);
            Ok(())
        })?;
        Ok((stats, records))
    }

    fn raw(value: &str) -> FieldValue {
        FieldValue::Raw(value.to_owned())
    }

    #[test]
    fn csv_reads_quoted_values_and_blanks() {
        let data = "id,name,note\n1,\"Doe, Jane\",\"say \"\"hi\"\"\"\n2,Bob,\n";
        let (stats, records) = read(DataSourceKind::Csv, data).unwrap();
        assert_eq!(stats.record_count, 2);
        assert_eq!(stats.fields, ["id", "name", "note"]);
        assert_eq!(records[0]["name"], raw("Doe, Jane"));
        assert_eq!(records[0]["note"], raw("say \"hi\""));
        assert_eq!(records[1]["note"], FieldValue::Null);
    }

    #[test]
    fn csv_rejects_bad_headers_and_rows() {
        let cases = [
            ("", "no header"),
            ("id,id\n1,2\n", "duplicate"),
            ("id, \n1,2\n", "empty header"),
            ("id,name\n1\n", "fields but the header"),
        ];
        for (data, needle) in cases {
            let err = read(DataSourceKind::Csv, data).unwrap_err().to_string();
            assert!(err.contains(needle), "{data:?}: {err}");
        }
    }

    #[test]
    fn csv_rejects_invalid_utf8() {
        let bytes: &[u8] = b"id,name\n1,\xff\xfe\n";
        let err = for_each_record(
            DataSourceKind::Csv,
            bytes,
            &Limits::default(),
            &mut |_, _| Ok(()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("UTF-8"), "{err}");
    }

    #[test]
    fn limits_are_enforced() {
        let limits = Limits {
            max_records: 2,
            max_fields: 2,
            max_value_bytes: 4,
        };
        let too_many = read_with(DataSourceKind::Csv, "a\n1\n2\n3\n", &limits).unwrap_err();
        assert!(matches!(too_many, IngestionError::LimitExceeded(_)));
        let too_wide = read_with(DataSourceKind::Csv, "a,b,c\n1,2,3\n", &limits).unwrap_err();
        assert!(matches!(too_wide, IngestionError::LimitExceeded(_)));
        let too_long = read_with(DataSourceKind::Csv, "a\nhello\n", &limits).unwrap_err();
        assert!(matches!(too_long, IngestionError::LimitExceeded(_)));
        let json = read_with(
            DataSourceKind::Json,
            r#"[{"a":1},{"a":2},{"a":3}]"#,
            &limits,
        )
        .unwrap_err();
        assert!(matches!(json, IngestionError::LimitExceeded(_)));
    }

    #[test]
    fn json_reads_flat_objects() {
        let data = r#"[{"id":"C1","age":30,"score":0.5,"vip":true,"tags":["a","b"],"x":null},{"id":"C2"}]"#;
        let (stats, records) = read(DataSourceKind::Json, data).unwrap();
        assert_eq!(stats.record_count, 2);
        assert_eq!(stats.fields, ["age", "id", "score", "tags", "vip", "x"]);
        assert_eq!(records[0]["age"], FieldValue::Integer(30));
        assert_eq!(records[0]["score"], FieldValue::Float(0.5));
        assert_eq!(records[0]["vip"], FieldValue::Bool(true));
        assert_eq!(
            records[0]["tags"],
            FieldValue::StringArray(vec!["a".into(), "b".into()])
        );
        assert_eq!(records[0]["x"], FieldValue::Null);
        assert!(!records[1].contains_key("age"));
    }

    #[test]
    fn json_empty_array_has_no_records() {
        let (stats, records) = read(DataSourceKind::Json, "[]").unwrap();
        assert_eq!(stats.record_count, 0);
        assert!(records.is_empty());
    }

    #[test]
    fn json_rejects_unsupported_shapes() {
        let cases = [
            (r#"{"id":1}"#, "top-level JSON array"),
            ("42", "top-level JSON array"),
            (r#"[{"id":1}, 5]"#, "record 2 is not a JSON object"),
            (r#"[{"a":{"b":1}}]"#, "nested object"),
            (r#"[{"a":[1,2]}]"#, "only arrays of strings"),
            (r#"[{"a":1}"#, "top-level JSON array"),
        ];
        for (data, needle) in cases {
            let err = read(DataSourceKind::Json, data).unwrap_err().to_string();
            assert!(err.contains(needle), "{data}: {err}");
        }
    }

    #[test]
    fn json_errors_do_not_quote_data() {
        let err = read(DataSourceKind::Json, r#""top-secret-value""#)
            .unwrap_err()
            .to_string();
        assert!(!err.contains("top-secret-value"), "{err}");
    }
}
