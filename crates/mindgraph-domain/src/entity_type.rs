//! Entity types (ADR-003).

use crate::text::{MAX_DESCRIPTION_CHARS, MAX_DISPLAY_TEXT_CHARS, optional_text, required_text};
use crate::{DomainError, EntityTypeId, GraphId, PropertiesSchema, TypeName};

/// A kind of entity in a graph's ontology, for example `Customer`, `Account`
/// or `Device`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityType {
    id: EntityTypeId,
    graph_id: GraphId,
    name: TypeName,
    display_name: String,
    description: Option<String>,
    properties_schema: PropertiesSchema,
}

impl EntityType {
    /// Creates an entity type. The display name defaults to `name` and there
    /// is no description. Use [`Self::with_display_name`] and
    /// [`Self::with_description`] to set them.
    #[must_use]
    pub fn new(
        id: EntityTypeId,
        graph_id: GraphId,
        name: TypeName,
        properties_schema: PropertiesSchema,
    ) -> Self {
        Self {
            id,
            graph_id,
            display_name: name.to_string(),
            name,
            description: None,
            properties_schema,
        }
    }

    /// Sets the human-readable name, for example `"IP Address"`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::EmptyValue`] or [`DomainError::TooLong`] (more
    /// than 120 characters).
    pub fn with_display_name(mut self, display_name: &str) -> Result<Self, DomainError> {
        self.display_name = required_text("display name", display_name, MAX_DISPLAY_TEXT_CHARS)?;
        Ok(self)
    }

    /// Sets the description. Blank text clears it.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::TooLong`] for more than 2,000 characters.
    pub fn with_description(mut self, description: &str) -> Result<Self, DomainError> {
        self.description = optional_text(
            "entity type description",
            Some(description),
            MAX_DESCRIPTION_CHARS,
        )?;
        Ok(self)
    }

    /// The entity type identifier.
    #[must_use]
    pub const fn id(&self) -> EntityTypeId {
        self.id
    }

    /// The graph this entity type belongs to.
    #[must_use]
    pub const fn graph_id(&self) -> GraphId {
        self.graph_id
    }

    /// The machine name, unique within an ontology.
    #[must_use]
    pub const fn name(&self) -> &TypeName {
        &self.name
    }

    /// The human-readable name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// The optional description.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The properties every entity of this type may or must carry.
    #[must_use]
    pub const fn properties_schema(&self) -> &PropertiesSchema {
        &self.properties_schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> EntityType {
        EntityType::new(
            EntityTypeId::new(),
            GraphId::new(),
            TypeName::new("IPAddress").unwrap(),
            PropertiesSchema::default(),
        )
    }

    #[test]
    fn display_name_defaults_to_the_machine_name() {
        let entity_type = device();
        assert_eq!(entity_type.name().as_str(), "IPAddress");
        assert_eq!(entity_type.display_name(), "IPAddress");
        assert_eq!(entity_type.description(), None);
    }

    #[test]
    fn display_name_and_description_can_be_set() {
        let entity_type = device()
            .with_display_name(" IP Address ")
            .unwrap()
            .with_description("An IPv4 or IPv6 address")
            .unwrap();
        assert_eq!(entity_type.display_name(), "IP Address");
        assert_eq!(entity_type.description(), Some("An IPv4 or IPv6 address"));
    }

    #[test]
    fn entity_type_names_cannot_be_empty() {
        assert_eq!(
            TypeName::new(""),
            Err(DomainError::EmptyValue { field: "type name" })
        );
    }

    #[test]
    fn display_names_cannot_be_empty() {
        assert_eq!(
            device().with_display_name("  "),
            Err(DomainError::EmptyValue {
                field: "display name"
            })
        );
    }

    #[test]
    fn descriptions_have_a_length_limit() {
        assert_eq!(
            device().with_description(&"x".repeat(2_001)),
            Err(DomainError::TooLong {
                field: "entity type description",
                max: 2_000
            })
        );
    }
}
