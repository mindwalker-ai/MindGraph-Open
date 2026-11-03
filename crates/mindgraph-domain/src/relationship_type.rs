//! Relationship types (ADR-003).

use crate::{GraphId, PropertiesSchema, RelationshipTypeId, TypeName};

/// A kind of directed relationship between two entity types, for example
/// `Customer ──OWNS──> Account`.
///
/// The source and target entity types are referenced by name. Whether they
/// exist is checked when the relationship type is added to an
/// [`Ontology`](crate::Ontology).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipType {
    id: RelationshipTypeId,
    graph_id: GraphId,
    name: TypeName,
    source_entity_type: TypeName,
    target_entity_type: TypeName,
    properties_schema: PropertiesSchema,
}

impl RelationshipType {
    /// Creates a relationship type from `source_entity_type` to
    /// `target_entity_type`. Both may name the same entity type, for example
    /// `Account ──TRANSFERRED_TO──> Account`.
    #[must_use]
    pub const fn new(
        id: RelationshipTypeId,
        graph_id: GraphId,
        name: TypeName,
        source_entity_type: TypeName,
        target_entity_type: TypeName,
        properties_schema: PropertiesSchema,
    ) -> Self {
        Self {
            id,
            graph_id,
            name,
            source_entity_type,
            target_entity_type,
            properties_schema,
        }
    }

    /// The relationship type identifier.
    #[must_use]
    pub const fn id(&self) -> RelationshipTypeId {
        self.id
    }

    /// The graph this relationship type belongs to.
    #[must_use]
    pub const fn graph_id(&self) -> GraphId {
        self.graph_id
    }

    /// The machine name, unique within an ontology.
    #[must_use]
    pub const fn name(&self) -> &TypeName {
        &self.name
    }

    /// The entity type relationships of this type start from.
    #[must_use]
    pub const fn source_entity_type(&self) -> &TypeName {
        &self.source_entity_type
    }

    /// The entity type relationships of this type point to.
    #[must_use]
    pub const fn target_entity_type(&self) -> &TypeName {
        &self.target_entity_type
    }

    /// The properties every relationship of this type may or must carry.
    #[must_use]
    pub const fn properties_schema(&self) -> &PropertiesSchema {
        &self.properties_schema
    }
}
