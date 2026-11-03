//! Validated machine names for ontology types and property keys.
//!
//! Machine names end up as graph-store labels and property keys, so they
//! follow a conservative identifier format that every backend accepts:
//!
//! - 1 to 64 characters
//! - starts with an ASCII letter
//! - contains only ASCII letters, digits and `_`
//!
//! Names are case-sensitive. Human-friendly labels such as `"IP Address"`
//! belong in [`EntityType::display_name`](crate::EntityType::display_name).

use std::borrow::Borrow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::DomainError;

/// Maximum length of a machine name.
const MAX_NAME_CHARS: usize = 64;

fn validate_machine_name(field: &'static str, value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::EmptyValue { field });
    }
    if value.chars().count() > MAX_NAME_CHARS {
        return Err(DomainError::TooLong {
            field,
            max: MAX_NAME_CHARS,
        });
    }
    let invalid = |reason| DomainError::InvalidName {
        field,
        value: value.to_owned(),
        reason,
    };
    let mut chars = value.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return Err(invalid("must start with an ASCII letter"));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(invalid(
            "may only contain ASCII letters, digits and underscores",
        ));
    }
    Ok(())
}

/// The machine name of an entity type or relationship type, for example
/// `Customer` or `TRANSFERRED_TO`.
///
/// See the [module documentation](self) for the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TypeName(String);

impl TypeName {
    /// Parses a type name.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::EmptyValue`], [`DomainError::TooLong`] or
    /// [`DomainError::InvalidName`] if the value breaks a naming rule.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_machine_name("type name", &value)?;
        Ok(Self(value))
    }

    /// Returns the name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The key of a property on an entity or relationship, for example
/// `customer_id` or `email`.
///
/// See the [module documentation](self) for the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PropertyKey(String);

impl PropertyKey {
    /// Parses a property key.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::EmptyValue`], [`DomainError::TooLong`] or
    /// [`DomainError::InvalidName`] if the value breaks a naming rule.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_machine_name("property key", &value)?;
        Ok(Self(value))
    }

    /// Returns the key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! impl_name_traits {
    ($name:ident) => {
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        // Sound because `Eq`, `Ord` and `Hash` are derived from the inner `String`.
        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = DomainError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(name: $name) -> Self {
                name.0
            }
        }
    };
}

impl_name_traits!(TypeName);
impl_name_traits!(PropertyKey);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_typical_names() {
        for name in ["Customer", "IPAddress", "TRANSFERRED_TO", "Device2", "a"] {
            assert_eq!(TypeName::new(name).unwrap().as_str(), name);
        }
        assert_eq!(
            PropertyKey::new("customer_id").unwrap().as_str(),
            "customer_id"
        );
    }

    #[test]
    fn empty_and_blank_names_are_rejected() {
        for value in ["", "   ", "\t"] {
            assert_eq!(
                TypeName::new(value),
                Err(DomainError::EmptyValue { field: "type name" })
            );
            assert_eq!(
                PropertyKey::new(value),
                Err(DomainError::EmptyValue {
                    field: "property key"
                })
            );
        }
    }

    #[test]
    fn names_must_start_with_a_letter() {
        for value in ["1Customer", "_internal", " Customer"] {
            let err = TypeName::new(value).unwrap_err();
            assert!(
                matches!(
                    err,
                    DomainError::InvalidName {
                        reason: "must start with an ASCII letter",
                        ..
                    }
                ),
                "{value:?} gave {err:?}"
            );
        }
    }

    #[test]
    fn names_reject_spaces_punctuation_and_non_ascii() {
        for value in ["IP Address", "customer-id", "email!", "Café"] {
            assert!(
                matches!(
                    PropertyKey::new(value),
                    Err(DomainError::InvalidName { .. })
                ),
                "{value:?} should be rejected"
            );
        }
    }

    #[test]
    fn names_have_a_length_limit() {
        assert!(TypeName::new("a".repeat(64)).is_ok());
        assert_eq!(
            TypeName::new("a".repeat(65)),
            Err(DomainError::TooLong {
                field: "type name",
                max: 64
            })
        );
    }

    #[test]
    fn names_are_case_sensitive() {
        assert_ne!(
            TypeName::new("Customer").unwrap(),
            TypeName::new("customer").unwrap()
        );
    }

    #[test]
    fn serde_validates_names() {
        let name: TypeName = serde_json::from_str("\"OWNS\"").unwrap();
        assert_eq!(name.as_str(), "OWNS");
        assert_eq!(serde_json::to_string(&name).unwrap(), "\"OWNS\"");
        assert!(serde_json::from_str::<TypeName>("\"\"").is_err());
        assert!(serde_json::from_str::<PropertyKey>("\"has space\"").is_err());
    }
}
