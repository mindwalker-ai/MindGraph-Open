//! Entities (ADR-003).

use crate::{EntityId, GraphId, Properties, TypeName};

/// A node in a knowledge graph, for example one customer or one device.
///
/// Construction only guarantees structural validity: a valid id, type name
/// and properties. Whether the entity conforms to a schema is checked with
/// [`Ontology::validate_entity`](crate::Ontology::validate_entity), because
/// the same entity can be checked against different ontology versions.
///
/// Identifiers from source systems, such as `CUST-10291`, are ordinary
/// properties. They are never used as the [`EntityId`].
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    id: EntityId,
    graph_id: GraphId,
    entity_type: TypeName,
    properties: Properties,
}

impl Entity {
    /// Creates an entity.
    #[must_use]
    pub const fn new(
        id: EntityId,
        graph_id: GraphId,
        entity_type: TypeName,
        properties: Properties,
    ) -> Self {
        Self {
            id,
            graph_id,
            entity_type,
            properties,
        }
    }

    /// The entity identifier.
    #[must_use]
    pub const fn id(&self) -> EntityId {
        self.id
    }

    /// The graph this entity belongs to.
    #[must_use]
    pub const fn graph_id(&self) -> GraphId {
        self.graph_id
    }

    /// The name of this entity's [`EntityType`](crate::EntityType).
    #[must_use]
    pub const fn entity_type(&self) -> &TypeName {
        &self.entity_type
    }

    /// The entity's property values.
    #[must_use]
    pub const fn properties(&self) -> &Properties {
        &self.properties
    }
}
