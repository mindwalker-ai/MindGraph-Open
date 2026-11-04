//! JSON-compatible property values (ADR-003).
//!
//! v0.1 supports exactly six primitive types: string, integer, float,
//! boolean, datetime and string array. Every value maps to plain JSON with
//! [`Properties::to_json`]. Reading JSON back needs a
//! [`PropertiesSchema`] ([`Properties::from_json`]) because JSON alone cannot
//! tell a datetime from a string or a float `1.0` from an integer `1`.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DomainError, PropertiesSchema, PropertyKey};

/// The primitive type of a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyType {
    /// UTF-8 text.
    String,
    /// A 64-bit signed integer.
    Integer,
    /// A finite 64-bit floating point number.
    Float,
    /// `true` or `false`.
    Boolean,
    /// A point in time, normalised to UTC and written as RFC 3339.
    #[serde(rename = "datetime")]
    DateTime,
    /// A list of UTF-8 strings.
    StringArray,
}

impl PropertyType {
    /// Returns the canonical name of the type, as used in serialised schemas.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::Boolean => "boolean",
            Self::DateTime => "datetime",
            Self::StringArray => "string_array",
        }
    }
}

impl fmt::Display for PropertyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single property value.
///
/// A [`PropertyValue::Float`] must be finite. [`Properties::insert`] enforces
/// this, because JSON cannot represent `NaN` or infinity.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// UTF-8 text.
    String(String),
    /// A 64-bit signed integer.
    Integer(i64),
    /// A finite 64-bit floating point number.
    Float(f64),
    /// `true` or `false`.
    Boolean(bool),
    /// A point in time in UTC.
    DateTime(DateTime<Utc>),
    /// A list of UTF-8 strings.
    StringArray(Vec<String>),
}

impl PropertyValue {
    /// Returns the [`PropertyType`] of this value.
    #[must_use]
    pub const fn property_type(&self) -> PropertyType {
        match self {
            Self::String(_) => PropertyType::String,
            Self::Integer(_) => PropertyType::Integer,
            Self::Float(_) => PropertyType::Float,
            Self::Boolean(_) => PropertyType::Boolean,
            Self::DateTime(_) => PropertyType::DateTime,
            Self::StringArray(_) => PropertyType::StringArray,
        }
    }

    /// Converts the value to plain JSON.
    ///
    /// Datetimes become RFC 3339 strings in UTC (`2026-11-03T08:30:00Z`).
    /// A non-finite float, which [`Properties`] never contains, becomes
    /// `null`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::String(value) => Value::String(value.clone()),
            Self::Integer(value) => Value::from(*value),
            Self::Float(value) => {
                serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number)
            }
            Self::Boolean(value) => Value::Bool(*value),
            Self::DateTime(value) => {
                Value::String(value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
            }
            Self::StringArray(values) => {
                Value::Array(values.iter().cloned().map(Value::String).collect())
            }
        }
    }

    /// Decodes a non-null JSON value as `expected` (`key` names the property
    /// in errors), for example an exact-match search filter.
    ///
    /// Integers are accepted where a float is expected. Datetimes must be
    /// RFC 3339 strings and are normalised to UTC.
    ///
    /// # Errors
    ///
    /// [`DomainError::PropertyTypeMismatch`] if the JSON kind does not fit
    /// `expected`, or [`DomainError::InvalidPropertyValue`] for an integer out
    /// of range or an invalid datetime.
    pub fn from_json(
        key: &str,
        value: &Value,
        expected: PropertyType,
    ) -> Result<Self, DomainError> {
        let mismatch = |found: String| DomainError::PropertyTypeMismatch {
            key: key.to_owned(),
            expected,
            found,
        };
        let invalid = |reason: String| DomainError::InvalidPropertyValue {
            key: key.to_owned(),
            reason,
        };

        match (expected, value) {
            (PropertyType::String, Value::String(text)) => Ok(Self::String(text.clone())),
            (PropertyType::Integer, Value::Number(number)) => {
                if let Some(integer) = number.as_i64() {
                    Ok(Self::Integer(integer))
                } else if number.is_u64() {
                    Err(invalid(
                        "integer is out of range for a 64-bit signed integer".to_owned(),
                    ))
                } else {
                    Err(mismatch(json_kind(value).to_owned()))
                }
            }
            (PropertyType::Float, Value::Number(number)) => number
                .as_f64()
                .map(Self::Float)
                .ok_or_else(|| invalid("number cannot be represented as a float".to_owned())),
            (PropertyType::Boolean, Value::Bool(flag)) => Ok(Self::Boolean(*flag)),
            (PropertyType::DateTime, Value::String(text)) => DateTime::parse_from_rfc3339(text)
                .map(|datetime| Self::DateTime(datetime.with_timezone(&Utc)))
                .map_err(|err| invalid(format!("expected an RFC 3339 datetime: {err}"))),
            (PropertyType::StringArray, Value::Array(items)) => items
                .iter()
                .map(|item| match item {
                    Value::String(text) => Ok(text.clone()),
                    other => Err(mismatch(format!("array containing {}", json_kind(other)))),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Self::StringArray),
            (_, other) => Err(mismatch(json_kind(other).to_owned())),
        }
    }

    fn validate(&self, key: &PropertyKey) -> Result<(), DomainError> {
        match self {
            Self::Float(value) if !value.is_finite() => Err(DomainError::InvalidPropertyValue {
                key: key.to_string(),
                reason: "float must be finite".to_owned(),
            }),
            _ => Ok(()),
        }
    }
}

/// Describes a JSON value for error messages.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_f64() => "float",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

impl From<String> for PropertyValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for PropertyValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<i64> for PropertyValue {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<f64> for PropertyValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<bool> for PropertyValue {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}

impl From<DateTime<Utc>> for PropertyValue {
    fn from(value: DateTime<Utc>) -> Self {
        Self::DateTime(value)
    }
}

impl From<Vec<String>> for PropertyValue {
    fn from(values: Vec<String>) -> Self {
        Self::StringArray(values)
    }
}

/// The named property values of an entity or relationship.
///
/// Keys are unique and iterate in sorted order, so serialisation is
/// deterministic. Every stored value is valid: floats are always finite.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Properties(BTreeMap<PropertyKey, PropertyValue>);

impl Properties {
    /// Creates an empty set of properties.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a value and returns the previous value for `key`, if any.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidPropertyValue`] if the value is a
    /// non-finite float.
    pub fn insert(
        &mut self,
        key: PropertyKey,
        value: impl Into<PropertyValue>,
    ) -> Result<Option<PropertyValue>, DomainError> {
        let value = value.into();
        value.validate(&key)?;
        Ok(self.0.insert(key, value))
    }

    /// Returns the value stored under `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&PropertyValue> {
        self.0.get(key)
    }

    /// Returns `true` if a value is stored under `key`.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// Iterates over the properties in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&PropertyKey, &PropertyValue)> {
        self.0.iter()
    }

    /// Returns the number of properties.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if there are no properties.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Converts the properties to a JSON object.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_json()))
                .collect(),
        )
    }

    /// Decodes a JSON object using `schema` to determine each property's type.
    ///
    /// `null` values are treated as absent. The result is validated against
    /// `schema`, so unknown keys and missing required properties are errors.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::PropertiesNotAnObject`] if `value` is not an
    /// object, or the first property error found.
    pub fn from_json(value: &Value, schema: &PropertiesSchema) -> Result<Self, DomainError> {
        let Value::Object(object) = value else {
            return Err(DomainError::PropertiesNotAnObject);
        };

        let mut properties = Self::new();
        for (key, value) in object {
            let definition = schema
                .get(key)
                .ok_or_else(|| DomainError::UnknownProperty(key.clone()))?;
            if value.is_null() {
                continue;
            }
            let value = PropertyValue::from_json(key, value, definition.property_type)?;
            properties.insert(definition.key.clone(), value)?;
        }
        schema.validate(&properties)?;
        Ok(properties)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::PropertyDefinition;

    fn key(value: &str) -> PropertyKey {
        PropertyKey::new(value).unwrap()
    }

    fn customer_schema() -> PropertiesSchema {
        PropertiesSchema::new(vec![
            PropertyDefinition::required(key("name"), PropertyType::String),
            PropertyDefinition::optional(key("age"), PropertyType::Integer),
            PropertyDefinition::optional(key("risk_score"), PropertyType::Float),
            PropertyDefinition::optional(key("verified"), PropertyType::Boolean),
            PropertyDefinition::optional(key("joined_at"), PropertyType::DateTime),
            PropertyDefinition::optional(key("emails"), PropertyType::StringArray),
        ])
        .unwrap()
    }

    #[test]
    fn values_report_their_type() {
        let cases = [
            (PropertyValue::from("text"), PropertyType::String),
            (PropertyValue::from(42_i64), PropertyType::Integer),
            (PropertyValue::from(0.5), PropertyType::Float),
            (PropertyValue::from(true), PropertyType::Boolean),
            (PropertyValue::from(Utc::now()), PropertyType::DateTime),
            (
                PropertyValue::from(vec!["a".to_owned()]),
                PropertyType::StringArray,
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(value.property_type(), expected);
        }
    }

    #[test]
    fn property_type_names_match_serde() {
        for ty in [
            PropertyType::String,
            PropertyType::Integer,
            PropertyType::Float,
            PropertyType::Boolean,
            PropertyType::DateTime,
            PropertyType::StringArray,
        ] {
            let json = serde_json::to_string(&ty).unwrap();
            assert_eq!(json, format!("\"{ty}\""));
            assert_eq!(serde_json::from_str::<PropertyType>(&json).unwrap(), ty);
        }
    }

    #[test]
    fn every_type_converts_to_plain_json() {
        let joined = Utc.with_ymd_and_hms(2026, 11, 3, 8, 30, 0).unwrap();
        let mut properties = Properties::new();
        properties.insert(key("name"), "Ikhwan Nasution").unwrap();
        properties.insert(key("age"), 34_i64).unwrap();
        properties.insert(key("risk_score"), 0.91).unwrap();
        properties.insert(key("verified"), false).unwrap();
        properties.insert(key("joined_at"), joined).unwrap();
        properties
            .insert(key("emails"), vec!["ikhwan@example.com".to_owned()])
            .unwrap();

        assert_eq!(
            properties.to_json(),
            json!({
                "age": 34,
                "emails": ["ikhwan@example.com"],
                "joined_at": "2026-11-03T08:30:00Z",
                "name": "Ikhwan Nasution",
                "risk_score": 0.91,
                "verified": false,
            })
        );
    }

    #[test]
    fn json_round_trips_through_the_schema() {
        let input = json!({
            "name": "Ikhwan Nasution",
            "age": 34,
            "risk_score": 0.91,
            "verified": true,
            "joined_at": "2026-11-03T08:30:00Z",
            "emails": ["a@example.com", "b@example.com"],
        });
        let properties = Properties::from_json(&input, &customer_schema()).unwrap();
        assert_eq!(properties.len(), 6);
        assert_eq!(properties.to_json(), input);
    }

    #[test]
    fn iteration_order_is_sorted_by_key() {
        let mut properties = Properties::new();
        properties.insert(key("zeta"), 1_i64).unwrap();
        properties.insert(key("alpha"), 2_i64).unwrap();
        let keys: Vec<&str> = properties.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["alpha", "zeta"]);
    }

    #[test]
    fn insert_replaces_and_returns_previous_value() {
        let mut properties = Properties::new();
        assert_eq!(properties.insert(key("name"), "A").unwrap(), None);
        assert_eq!(
            properties.insert(key("name"), "B").unwrap(),
            Some(PropertyValue::from("A"))
        );
        assert_eq!(properties.get("name"), Some(&PropertyValue::from("B")));
    }

    #[test]
    fn non_finite_floats_are_rejected() {
        let mut properties = Properties::new();
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                properties.insert(key("score"), value),
                Err(DomainError::InvalidPropertyValue {
                    key: "score".to_owned(),
                    reason: "float must be finite".to_owned()
                })
            );
        }
        assert!(properties.is_empty());
    }

    #[test]
    fn datetimes_are_normalised_to_utc() {
        let input = json!({ "name": "A", "joined_at": "2026-11-03T15:30:00+07:00" });
        let properties = Properties::from_json(&input, &customer_schema()).unwrap();
        assert_eq!(
            properties.get("joined_at"),
            Some(&PropertyValue::DateTime(
                Utc.with_ymd_and_hms(2026, 11, 3, 8, 30, 0).unwrap()
            ))
        );
    }

    #[test]
    fn malformed_datetimes_are_rejected() {
        let input = json!({ "name": "A", "joined_at": "03/11/2026" });
        let err = Properties::from_json(&input, &customer_schema()).unwrap_err();
        assert!(matches!(
            err,
            DomainError::InvalidPropertyValue { ref key, .. } if key == "joined_at"
        ));
    }

    #[test]
    fn integers_are_accepted_as_floats_but_not_vice_versa() {
        let schema = customer_schema();
        let properties =
            Properties::from_json(&json!({ "name": "A", "risk_score": 1 }), &schema).unwrap();
        assert_eq!(
            properties.get("risk_score"),
            Some(&PropertyValue::Float(1.0))
        );

        let err = Properties::from_json(&json!({ "name": "A", "age": 1.5 }), &schema).unwrap_err();
        assert_eq!(
            err,
            DomainError::PropertyTypeMismatch {
                key: "age".to_owned(),
                expected: PropertyType::Integer,
                found: "float".to_owned()
            }
        );
    }

    #[test]
    fn integers_outside_i64_are_rejected() {
        let input = json!({ "name": "A", "age": u64::MAX });
        let err = Properties::from_json(&input, &customer_schema()).unwrap_err();
        assert!(matches!(err, DomainError::InvalidPropertyValue { ref key, .. } if key == "age"));
    }

    #[test]
    fn type_mismatches_are_reported() {
        let schema = customer_schema();
        let cases = [
            (
                json!({ "name": 7 }),
                "name",
                PropertyType::String,
                "integer",
            ),
            (
                json!({ "name": "A", "verified": "yes" }),
                "verified",
                PropertyType::Boolean,
                "string",
            ),
            (
                json!({ "name": "A", "emails": "a@example.com" }),
                "emails",
                PropertyType::StringArray,
                "string",
            ),
            (
                json!({ "name": "A", "emails": ["a", 1] }),
                "emails",
                PropertyType::StringArray,
                "array containing integer",
            ),
            (
                json!({ "name": { "first": "A" } }),
                "name",
                PropertyType::String,
                "object",
            ),
        ];
        for (input, key, expected, found) in cases {
            assert_eq!(
                Properties::from_json(&input, &schema),
                Err(DomainError::PropertyTypeMismatch {
                    key: key.to_owned(),
                    expected,
                    found: found.to_owned()
                }),
                "input: {input}"
            );
        }
    }

    #[test]
    fn nulls_are_treated_as_absent() {
        let schema = customer_schema();
        let properties =
            Properties::from_json(&json!({ "name": "A", "age": null }), &schema).unwrap();
        assert!(!properties.contains_key("age"));

        let err = Properties::from_json(&json!({ "name": null }), &schema).unwrap_err();
        assert_eq!(err, DomainError::MissingRequiredProperty("name".to_owned()));
    }

    #[test]
    fn unknown_json_keys_are_rejected() {
        let err =
            Properties::from_json(&json!({ "name": "A", "nickname": "B" }), &customer_schema())
                .unwrap_err();
        assert_eq!(err, DomainError::UnknownProperty("nickname".to_owned()));
    }

    #[test]
    fn non_object_json_is_rejected() {
        let err = Properties::from_json(&json!(["name"]), &customer_schema()).unwrap_err();
        assert_eq!(err, DomainError::PropertiesNotAnObject);
    }
}
