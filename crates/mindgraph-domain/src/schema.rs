//! Property schemas for entity types and relationship types.
//!
//! A schema is a flat list of typed property keys, each optionally required.
//! That is all v0.1 needs. There are deliberately no nested objects, default
//! values, enums or value constraints.

use std::collections::BTreeSet;

use crate::{DomainError, Properties, PropertyKey, PropertyType};

/// Declares one property of an entity type or relationship type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyDefinition {
    /// The property key.
    pub key: PropertyKey,
    /// The type every value of this property must have.
    pub property_type: PropertyType,
    /// Whether every entity or relationship must carry this property.
    pub required: bool,
}

impl PropertyDefinition {
    /// Declares a property that must always be present.
    #[must_use]
    pub const fn required(key: PropertyKey, property_type: PropertyType) -> Self {
        Self {
            key,
            property_type,
            required: true,
        }
    }

    /// Declares a property that may be absent.
    #[must_use]
    pub const fn optional(key: PropertyKey, property_type: PropertyType) -> Self {
        Self {
            key,
            property_type,
            required: false,
        }
    }
}

/// The declared properties of an entity type or relationship type.
///
/// Validation is strict. A property that is not declared is rejected, so an
/// empty schema accepts only empty [`Properties`]. Definitions keep their
/// insertion order so user interfaces can show them as authored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertiesSchema {
    definitions: Vec<PropertyDefinition>,
}

impl PropertiesSchema {
    /// Creates a schema from property definitions.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::DuplicateProperty`] if two definitions share a
    /// key.
    pub fn new(definitions: Vec<PropertyDefinition>) -> Result<Self, DomainError> {
        let mut seen = BTreeSet::new();
        for definition in &definitions {
            if !seen.insert(definition.key.as_str()) {
                return Err(DomainError::DuplicateProperty(definition.key.to_string()));
            }
        }
        Ok(Self { definitions })
    }

    /// Returns the definition for `key`, if declared.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&PropertyDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.key.as_str() == key)
    }

    /// Returns all definitions in declaration order.
    #[must_use]
    pub fn definitions(&self) -> &[PropertyDefinition] {
        &self.definitions
    }

    /// Returns `true` if no properties are declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    /// Checks that `properties` conform to this schema.
    ///
    /// Properties are checked in key order, then required properties in
    /// declaration order, so the first error reported is deterministic.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::UnknownProperty`],
    /// [`DomainError::PropertyTypeMismatch`] or
    /// [`DomainError::MissingRequiredProperty`].
    pub fn validate(&self, properties: &Properties) -> Result<(), DomainError> {
        for (key, value) in properties.iter() {
            let definition = self
                .get(key.as_str())
                .ok_or_else(|| DomainError::UnknownProperty(key.to_string()))?;
            if value.property_type() != definition.property_type {
                return Err(DomainError::PropertyTypeMismatch {
                    key: key.to_string(),
                    expected: definition.property_type,
                    found: value.property_type().to_string(),
                });
            }
        }
        for definition in self.definitions.iter().filter(|d| d.required) {
            if !properties.contains_key(definition.key.as_str()) {
                return Err(DomainError::MissingRequiredProperty(
                    definition.key.to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(value: &str) -> PropertyKey {
        PropertyKey::new(value).unwrap()
    }

    fn schema() -> PropertiesSchema {
        PropertiesSchema::new(vec![
            PropertyDefinition::required(key("customer_id"), PropertyType::String),
            PropertyDefinition::required(key("name"), PropertyType::String),
            PropertyDefinition::optional(key("age"), PropertyType::Integer),
        ])
        .unwrap()
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let err = PropertiesSchema::new(vec![
            PropertyDefinition::required(key("email"), PropertyType::String),
            PropertyDefinition::optional(key("email"), PropertyType::StringArray),
        ])
        .unwrap_err();
        assert_eq!(err, DomainError::DuplicateProperty("email".to_owned()));
    }

    #[test]
    fn definitions_keep_declaration_order() {
        let schema = schema();
        let keys: Vec<&str> = schema
            .definitions()
            .iter()
            .map(|d| d.key.as_str())
            .collect();
        assert_eq!(keys, ["customer_id", "name", "age"]);
    }

    #[test]
    fn conforming_properties_pass() {
        let mut properties = Properties::new();
        properties.insert(key("customer_id"), "CUST-10291").unwrap();
        properties.insert(key("name"), "Ikhwan").unwrap();
        assert_eq!(schema().validate(&properties), Ok(()));
    }

    #[test]
    fn missing_required_properties_are_reported_in_declaration_order() {
        let err = schema().validate(&Properties::new()).unwrap_err();
        assert_eq!(
            err,
            DomainError::MissingRequiredProperty("customer_id".to_owned())
        );
    }

    #[test]
    fn wrong_types_are_reported() {
        let mut properties = Properties::new();
        properties.insert(key("customer_id"), "CUST-1").unwrap();
        properties.insert(key("name"), "Ikhwan").unwrap();
        properties.insert(key("age"), "thirty").unwrap();
        assert_eq!(
            schema().validate(&properties),
            Err(DomainError::PropertyTypeMismatch {
                key: "age".to_owned(),
                expected: PropertyType::Integer,
                found: "string".to_owned()
            })
        );
    }

    #[test]
    fn undeclared_properties_are_rejected() {
        let mut properties = Properties::new();
        properties.insert(key("nickname"), "Ik").unwrap();
        assert_eq!(
            PropertiesSchema::default().validate(&properties),
            Err(DomainError::UnknownProperty("nickname".to_owned()))
        );
    }
}
