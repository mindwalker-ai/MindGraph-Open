//! Dataset profiling and primitive type inference.
//!
//! # Inference (deterministic, conservative)
//!
//! CSV text: blank → null; `true`/`false` (any case) → boolean; a number
//! with a leading zero (`007`, `00123`) → **string**, so identifiers keep
//! their zeros; valid `i64` → integer; finite decimal number → float; RFC
//! 3339 → datetime; anything else → string.
//!
//! JSON: boolean → boolean, integer → integer, other number → float, RFC
//! 3339 string → datetime, other string → string, array of strings →
//! string array. JSON strings are never re-interpreted as numbers.
//!
//! # Widening lattice
//!
//! Null never changes the type. Equal types stay. `integer` + `float` →
//! `float`. Any other mix → `string`. A field with no non-null value → `string`.

use std::collections::BTreeMap;
use std::io::Read;

use chrono::DateTime;
use mindgraph_domain::{DataSourceKind, PropertyType};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::IngestionError;
use crate::record::{FieldValue, Limits, Record, for_each_record};

/// Maximum number of sample rows kept in a profile.
pub const MAX_SAMPLE_ROWS: usize = 20;
/// Maximum number of distinct sample values kept per field.
pub const MAX_SAMPLE_VALUES: usize = 5;
/// Sample strings are truncated to this many characters.
const MAX_SAMPLE_CHARS: usize = 200;

/// The result of analysing a dataset. Contains a bounded number of sample
/// values so a UI can preview the data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetProfile {
    /// Number of records.
    pub record_count: u64,
    /// Fields in CSV header order, or sorted by name for JSON.
    pub fields: Vec<FieldProfile>,
    /// Up to [`MAX_SAMPLE_ROWS`] leading records, with long strings truncated.
    pub sample_rows: Vec<Map<String, Value>>,
}

impl DatasetProfile {
    /// The field names, in profile order.
    #[must_use]
    pub fn field_names(&self) -> Vec<String> {
        self.fields.iter().map(|field| field.name.clone()).collect()
    }
}

/// The profile of one field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldProfile {
    /// The field name.
    pub name: String,
    /// The inferred primitive type.
    pub inferred_type: PropertyType,
    /// Whether some record has no value for this field.
    pub nullable: bool,
    /// Number of records with a value.
    pub non_null_count: u64,
    /// Up to [`MAX_SAMPLE_VALUES`] distinct values, truncated.
    pub sample_values: Vec<String>,
}

pub(crate) fn looks_numeric(text: &str) -> bool {
    text.bytes().any(|b| b.is_ascii_digit())
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E'))
}

fn has_leading_zero(text: &str) -> bool {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text).as_bytes();
    digits.len() > 1 && digits[0] == b'0' && digits[1].is_ascii_digit()
}

fn is_datetime(text: &str) -> bool {
    DateTime::parse_from_rfc3339(text.trim()).is_ok()
}

fn infer_text(text: &str) -> PropertyType {
    let text = text.trim();
    if text.eq_ignore_ascii_case("true") || text.eq_ignore_ascii_case("false") {
        PropertyType::Boolean
    } else if has_leading_zero(text) {
        PropertyType::String
    } else if text.parse::<i64>().is_ok() {
        PropertyType::Integer
    } else if looks_numeric(text) && text.parse::<f64>().is_ok_and(f64::is_finite) {
        PropertyType::Float
    } else if is_datetime(text) {
        PropertyType::DateTime
    } else {
        PropertyType::String
    }
}

/// The type of one value, or `None` for null.
#[must_use]
pub fn infer_type(value: &FieldValue) -> Option<PropertyType> {
    match value {
        FieldValue::Null => None,
        FieldValue::Raw(text) => Some(infer_text(text)),
        FieldValue::Str(text) if is_datetime(text) => Some(PropertyType::DateTime),
        FieldValue::Str(_) => Some(PropertyType::String),
        FieldValue::Bool(_) => Some(PropertyType::Boolean),
        FieldValue::Integer(_) => Some(PropertyType::Integer),
        FieldValue::Float(_) => Some(PropertyType::Float),
        FieldValue::StringArray(_) => Some(PropertyType::StringArray),
    }
}

/// Combines the type seen so far with the type of the next value.
#[must_use]
pub fn widen(current: Option<PropertyType>, next: PropertyType) -> PropertyType {
    match current {
        None => next,
        Some(current) if current == next => current,
        Some(PropertyType::Integer | PropertyType::Float)
            if matches!(next, PropertyType::Integer | PropertyType::Float) =>
        {
            PropertyType::Float
        }
        Some(_) => PropertyType::String,
    }
}

fn truncate(text: &str) -> String {
    text.chars().take(MAX_SAMPLE_CHARS).collect()
}

fn sample_value(value: &FieldValue) -> Value {
    match value {
        FieldValue::Null => Value::Null,
        FieldValue::Raw(text) | FieldValue::Str(text) => Value::String(truncate(text)),
        FieldValue::Bool(flag) => Value::Bool(*flag),
        FieldValue::Integer(number) => Value::from(*number),
        FieldValue::Float(number) => {
            serde_json::Number::from_f64(*number).map_or(Value::Null, Value::Number)
        }
        FieldValue::StringArray(items) => Value::Array(
            items
                .iter()
                .map(|item| Value::String(truncate(item)))
                .collect(),
        ),
    }
}

fn sample_text(value: &FieldValue) -> Option<String> {
    match value {
        FieldValue::Null => None,
        FieldValue::Raw(text) | FieldValue::Str(text) => Some(truncate(text)),
        other => Some(truncate(&sample_value(other).to_string())),
    }
}

#[derive(Default)]
struct FieldState {
    inferred: Option<PropertyType>,
    non_null: u64,
    samples: Vec<String>,
}

/// Streams a dataset once and builds its profile. `progress` is called with
/// the number of records read every 1,000 records.
///
/// # Errors
///
/// Any parsing or limit error from reading the dataset.
pub fn profile_dataset(
    kind: DataSourceKind,
    reader: impl Read,
    limits: &Limits,
    progress: &mut dyn FnMut(u64),
) -> Result<DatasetProfile, IngestionError> {
    let mut states: BTreeMap<String, FieldState> = BTreeMap::new();
    let mut sample_rows = Vec::new();
    let stats = for_each_record(kind, reader, limits, &mut |number, record: Record| {
        if sample_rows.len() < MAX_SAMPLE_ROWS {
            sample_rows.push(
                record
                    .iter()
                    .map(|(k, v)| (k.clone(), sample_value(v)))
                    .collect(),
            );
        }
        for (name, value) in &record {
            let state = states.entry(name.clone()).or_default();
            if let Some(value_type) = infer_type(value) {
                state.inferred = Some(widen(state.inferred, value_type));
                state.non_null += 1;
                if state.samples.len() < MAX_SAMPLE_VALUES
                    && let Some(text) = sample_text(value)
                    && !state.samples.contains(&text)
                {
                    state.samples.push(text);
                }
            }
        }
        if number % 1_000 == 0 {
            progress(number);
        }
        Ok(())
    })?;
    let fields = stats
        .fields
        .iter()
        .map(|name| {
            let state = states.remove(name).unwrap_or_default();
            FieldProfile {
                name: name.clone(),
                inferred_type: state.inferred.unwrap_or(PropertyType::String),
                nullable: stats.record_count == 0 || state.non_null < stats.record_count,
                non_null_count: state.non_null,
                sample_values: state.samples,
            }
        })
        .collect();
    Ok(DatasetProfile {
        record_count: stats.record_count,
        fields,
        sample_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(text: &str) -> Option<PropertyType> {
        infer_type(&FieldValue::Raw(text.to_owned()))
    }

    fn profile(kind: DataSourceKind, data: &str) -> DatasetProfile {
        profile_dataset(kind, data.as_bytes(), &Limits::default(), &mut |_| {}).unwrap()
    }

    fn field<'a>(profile: &'a DatasetProfile, name: &str) -> &'a FieldProfile {
        profile.fields.iter().find(|f| f.name == name).unwrap()
    }

    #[test]
    fn csv_text_inference() {
        assert_eq!(raw("42"), Some(PropertyType::Integer));
        assert_eq!(raw("-7"), Some(PropertyType::Integer));
        assert_eq!(raw("3.14"), Some(PropertyType::Float));
        assert_eq!(raw("1e3"), Some(PropertyType::Float));
        assert_eq!(raw("TRUE"), Some(PropertyType::Boolean));
        assert_eq!(raw("2026-11-03T08:30:00Z"), Some(PropertyType::DateTime));
        assert_eq!(raw("Jakarta"), Some(PropertyType::String));
        assert_eq!(raw("NaN"), Some(PropertyType::String));
        assert_eq!(raw("inf"), Some(PropertyType::String));
        assert_eq!(
            raw("2026-11-03"),
            Some(PropertyType::String),
            "dates without time stay strings"
        );
    }

    #[test]
    fn leading_zero_identifiers_stay_strings() {
        assert_eq!(raw("00123"), Some(PropertyType::String));
        assert_eq!(raw("0000123456"), Some(PropertyType::String));
        assert_eq!(raw("007.5"), Some(PropertyType::String));
        assert_eq!(raw("0"), Some(PropertyType::Integer));
        assert_eq!(raw("0.5"), Some(PropertyType::Float));
    }

    #[test]
    fn widening_lattice() {
        use PropertyType::{Boolean, DateTime, Float, Integer, String};
        assert_eq!(widen(None, Integer), Integer);
        assert_eq!(widen(Some(Integer), Integer), Integer);
        assert_eq!(widen(Some(Integer), Float), Float);
        assert_eq!(widen(Some(Float), Integer), Float);
        assert_eq!(widen(Some(Integer), String), String);
        assert_eq!(widen(Some(Boolean), String), String);
        assert_eq!(widen(Some(DateTime), String), String);
        assert_eq!(widen(Some(Boolean), Integer), String);
    }

    #[test]
    fn csv_profile_handles_nulls_and_mixed_types() {
        let data = "id,age,score,city,empty,mixed\n001,30,1,Jakarta,,1\n002,,2.5,,,x\n003,41,3,Bandung,,2\n";
        let profile = profile(DataSourceKind::Csv, data);
        assert_eq!(profile.record_count, 3);
        let names: Vec<&str> = profile.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["id", "age", "score", "city", "empty", "mixed"]);
        assert_eq!(field(&profile, "id").inferred_type, PropertyType::String);
        let age = field(&profile, "age");
        assert_eq!(age.inferred_type, PropertyType::Integer);
        assert!(age.nullable);
        assert_eq!(age.non_null_count, 2);
        assert_eq!(field(&profile, "score").inferred_type, PropertyType::Float);
        let empty = field(&profile, "empty");
        assert_eq!(
            empty.inferred_type,
            PropertyType::String,
            "all-null resolves to string"
        );
        assert_eq!(empty.non_null_count, 0);
        assert_eq!(field(&profile, "mixed").inferred_type, PropertyType::String);
        assert_eq!(
            field(&profile, "city").sample_values,
            ["Jakarta", "Bandung"]
        );
    }

    #[test]
    fn json_profile_infers_native_types() {
        let data = r#"[
          {"id":"C1","n":1,"f":1,"when":"2026-11-03T08:30:00Z","tags":["a"],"ok":true,"z":null},
          {"id":"C2","n":2,"f":2.5,"when":"2026-11-04T08:30:00Z","tags":[],"ok":false}
        ]"#;
        let profile = profile(DataSourceKind::Json, data);
        assert_eq!(field(&profile, "id").inferred_type, PropertyType::String);
        assert_eq!(field(&profile, "n").inferred_type, PropertyType::Integer);
        assert_eq!(field(&profile, "f").inferred_type, PropertyType::Float);
        assert_eq!(
            field(&profile, "when").inferred_type,
            PropertyType::DateTime
        );
        assert_eq!(
            field(&profile, "tags").inferred_type,
            PropertyType::StringArray
        );
        assert_eq!(field(&profile, "ok").inferred_type, PropertyType::Boolean);
        let z = field(&profile, "z");
        assert_eq!(
            (z.inferred_type, z.nullable, z.non_null_count),
            (PropertyType::String, true, 0)
        );
    }

    #[test]
    fn json_strings_are_not_reinterpreted_as_numbers() {
        let profile = profile(DataSourceKind::Json, r#"[{"code":"123"},{"code":"456"}]"#);
        assert_eq!(field(&profile, "code").inferred_type, PropertyType::String);
    }

    #[test]
    fn samples_are_bounded() {
        let mut data = String::from("v\n");
        for i in 0..100 {
            data.push_str(&format!("{i}{}\n", "x".repeat(300)));
        }
        let profile = profile(DataSourceKind::Csv, &data);
        assert_eq!(profile.sample_rows.len(), MAX_SAMPLE_ROWS);
        let v = field(&profile, "v");
        assert_eq!(v.sample_values.len(), MAX_SAMPLE_VALUES);
        assert!(v.sample_values.iter().all(|s| s.chars().count() <= 200));
    }

    #[test]
    fn profiles_round_trip_through_json() {
        let profile = profile(DataSourceKind::Csv, "id,age\nC1,30\n");
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["fields"][1]["inferred_type"], "integer");
        assert_eq!(
            serde_json::from_value::<DatasetProfile>(json).unwrap(),
            profile
        );
    }
}
