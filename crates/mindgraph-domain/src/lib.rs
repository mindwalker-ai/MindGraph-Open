//! Infrastructure-independent domain model for MindGraph Open (ADR-003).
//!
//! This crate defines the core vocabulary of the platform (workspaces, graph
//! projects, ontologies, entities and relationships) together with their
//! validation rules. It deliberately knows nothing about HTTP, PostgreSQL,
//! HugeGraph or the Python AI service (ADR-002). Adapters translate to and
//! from these types at the edges of the system.
//!
//! # Conventions
//!
//! - Identifiers are strongly typed UUIDv7 values such as [`GraphId`] and
//!   [`EntityId`]. A nil or non-v7 UUID can never become an identifier.
//! - Constrained strings are parsed into newtypes ([`TypeName`],
//!   [`PropertyKey`]) so invalid values cannot travel through the domain.
//! - Constructors validate their input and return [`DomainError`] on failure.
//!   Validation is deterministic: the same input always yields the same result.
//! - Rules that span several objects (for example "a relationship's source
//!   entity has the type its relationship type requires") live on
//!   [`Ontology`], because entities are validated against a specific ontology
//!   version.
//!
//! # Example
//!
//! ```
//! use mindgraph_domain::{
//!     Entity, EntityId, EntityType, EntityTypeId, GraphId, Ontology, OntologyVersion,
//!     Properties, PropertiesSchema, PropertyDefinition, PropertyKey, PropertyType, TypeName,
//! };
//!
//! # fn main() -> Result<(), mindgraph_domain::DomainError> {
//! let graph_id = GraphId::new();
//! let schema = PropertiesSchema::new(vec![PropertyDefinition::required(
//!     PropertyKey::new("name")?,
//!     PropertyType::String,
//! )])?;
//! let customer =
//!     EntityType::new(EntityTypeId::new(), graph_id, TypeName::new("Customer")?, schema);
//! let ontology = Ontology::new(graph_id, OntologyVersion::INITIAL, vec![customer], vec![])?;
//!
//! let mut properties = Properties::new();
//! properties.insert(PropertyKey::new("name")?, "Ikhwan Nasution")?;
//! let entity = Entity::new(EntityId::new(), graph_id, TypeName::new("Customer")?, properties);
//!
//! ontology.validate_entity(&entity)?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod entity;
mod entity_type;
mod error;
mod graph_project;
mod id;
mod name;
mod ontology;
mod property;
mod relationship;
mod relationship_type;
mod schema;
mod text;
mod workspace;

pub use entity::Entity;
pub use entity_type::EntityType;
pub use error::DomainError;
pub use graph_project::{GraphProject, GraphStatus};
pub use id::{EntityId, EntityTypeId, GraphId, RelationshipId, RelationshipTypeId, WorkspaceId};
pub use name::{PropertyKey, TypeName};
pub use ontology::{Ontology, OntologyVersion};
pub use property::{Properties, PropertyType, PropertyValue};
pub use relationship::Relationship;
pub use relationship_type::RelationshipType;
pub use schema::{PropertiesSchema, PropertyDefinition};
pub use workspace::Workspace;
