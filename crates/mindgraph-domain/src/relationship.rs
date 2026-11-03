//! Relationships (ADR-003).

use crate::{EntityId, GraphId, Properties, RelationshipId, TypeName};

/// A directed edge between two entities, for example "customer A OWNS
/// account B".
///
/// The endpoints are always valid, non-empty [`EntityId`]s, guaranteed by
/// the type. Self-loops (`source_id == target_id`) are allowed. Whether the
/// endpoints have the entity types the relationship type requires is checked
/// with [`Ontology::validate_relationship`](crate::Ontology::validate_relationship).
#[derive(Debug, Clone, PartialEq)]
pub struct Relationship {
    id: RelationshipId,
    graph_id: GraphId,
    relationship_type: TypeName,
    source_id: EntityId,
    target_id: EntityId,
    properties: Properties,
}

impl Relationship {
    /// Creates a relationship from `source_id` to `target_id`.
    #[must_use]
    pub const fn new(
        id: RelationshipId,
        graph_id: GraphId,
        relationship_type: TypeName,
        source_id: EntityId,
        target_id: EntityId,
        properties: Properties,
    ) -> Self {
        Self {
            id,
            graph_id,
            relationship_type,
            source_id,
            target_id,
            properties,
        }
    }

    /// The relationship identifier.
    #[must_use]
    pub const fn id(&self) -> RelationshipId {
        self.id
    }

    /// The graph this relationship belongs to.
    #[must_use]
    pub const fn graph_id(&self) -> GraphId {
        self.graph_id
    }

    /// The name of this relationship's
    /// [`RelationshipType`](crate::RelationshipType).
    #[must_use]
    pub const fn relationship_type(&self) -> &TypeName {
        &self.relationship_type
    }

    /// The entity the relationship starts from.
    #[must_use]
    pub const fn source_id(&self) -> EntityId {
        self.source_id
    }

    /// The entity the relationship points to.
    #[must_use]
    pub const fn target_id(&self) -> EntityId {
        self.target_id
    }

    /// The relationship's property values.
    #[must_use]
    pub const fn properties(&self) -> &Properties {
        &self.properties
    }
}
