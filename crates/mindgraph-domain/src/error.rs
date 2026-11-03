//! The domain error type.

use crate::id::{EntityId, GraphId};
use crate::property::PropertyType;

/// An error raised when input violates a domain invariant.
///
/// Domain errors describe what is wrong in domain terms. They never wrap
/// infrastructure errors (database, network, HTTP): adapters report their
/// own failures separately and map these variants to API error codes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    /// An identifier could not be parsed or is not a valid MindGraph identifier.
    #[error("invalid {kind}: {reason}")]
    InvalidId {
        /// The kind of identifier, for example `"graph id"`.
        kind: &'static str,
        /// Why the identifier was rejected.
        reason: String,
    },

    /// A required value was empty or contained only whitespace.
    #[error("{field} must not be empty")]
    EmptyValue {
        /// The name of the offending field.
        field: &'static str,
    },

    /// A text value exceeded its maximum length.
    #[error("{field} must be at most {max} characters")]
    TooLong {
        /// The name of the offending field.
        field: &'static str,
        /// The maximum number of characters allowed.
        max: usize,
    },

    /// A machine name (type name or property key) has an invalid format.
    #[error("invalid {field} `{value}`: {reason}")]
    InvalidName {
        /// The name of the offending field.
        field: &'static str,
        /// The rejected value.
        value: String,
        /// Which naming rule was broken.
        reason: &'static str,
    },

    /// `updated_at` is earlier than `created_at`.
    #[error("updated_at must not be earlier than created_at")]
    InvalidTimestamps,

    /// A graph status string is not recognised.
    #[error("unknown graph status `{0}`")]
    UnknownGraphStatus(String),

    /// An ontology version number is out of range.
    #[error("invalid ontology version: {0}")]
    InvalidOntologyVersion(String),

    /// An object belongs to a different graph than the one it is used with.
    #[error("{kind} `{name}` belongs to graph {found}, expected graph {expected}")]
    GraphMismatch {
        /// The kind of object, for example `"entity type"`.
        kind: &'static str,
        /// The object's name or identifier.
        name: String,
        /// The graph the object was expected to belong to.
        expected: GraphId,
        /// The graph the object actually belongs to.
        found: GraphId,
    },

    /// Two entity types in one ontology share a name.
    #[error("entity type `{0}` is defined more than once")]
    DuplicateEntityType(String),

    /// Two relationship types in one ontology share a name.
    #[error("relationship type `{0}` is defined more than once")]
    DuplicateRelationshipType(String),

    /// Two property definitions in one schema share a key.
    #[error("property `{0}` is defined more than once")]
    DuplicateProperty(String),

    /// A referenced entity type does not exist in the ontology.
    #[error("entity type `{0}` does not exist in the ontology")]
    UnknownEntityType(String),

    /// A referenced relationship type does not exist in the ontology.
    #[error("relationship type `{0}` does not exist in the ontology")]
    UnknownRelationshipType(String),

    /// A relationship endpoint has a different entity type than its
    /// relationship type requires.
    #[error(
        "relationship type `{relationship_type}` requires {endpoint} entity type `{expected}`, \
         got `{found}`"
    )]
    EndpointTypeMismatch {
        /// The relationship type being validated.
        relationship_type: String,
        /// Either `"source"` or `"target"`.
        endpoint: &'static str,
        /// The entity type required by the relationship type.
        expected: String,
        /// The entity type of the supplied entity.
        found: String,
    },

    /// The entity supplied for an endpoint is not the one the relationship
    /// refers to.
    #[error("{endpoint} entity {found} does not match the relationship's {endpoint} id {expected}")]
    EndpointMismatch {
        /// Either `"source"` or `"target"`.
        endpoint: &'static str,
        /// The entity id stored on the relationship.
        expected: EntityId,
        /// The id of the entity that was supplied.
        found: EntityId,
    },

    /// A property is not declared in the schema it is validated against.
    #[error("property `{0}` is not defined in the schema")]
    UnknownProperty(String),

    /// A property marked as required is absent.
    #[error("required property `{0}` is missing")]
    MissingRequiredProperty(String),

    /// A property value has a different type than its definition declares.
    #[error("property `{key}` expects a {expected} value, got {found}")]
    PropertyTypeMismatch {
        /// The property key.
        key: String,
        /// The declared property type.
        expected: PropertyType,
        /// A description of the value that was supplied.
        found: String,
    },

    /// A property value has the right shape but an unusable value, for
    /// example a malformed timestamp or a non-finite float.
    #[error("property `{key}` has an invalid value: {reason}")]
    InvalidPropertyValue {
        /// The property key.
        key: String,
        /// Why the value was rejected.
        reason: String,
    },

    /// A JSON properties payload was not a JSON object.
    #[error("properties must be a JSON object")]
    PropertiesNotAnObject,
}
