//! Strongly typed UUIDv7 identifiers (ADR-003).
//!
//! Every MindGraph-owned object gets its own identifier type so that, for
//! example, an [`EntityId`] can never be passed where a [`GraphId`] is
//! expected. Identifiers are UUIDv7: globally unique, sortable by creation
//! time, and not sequential. External source identifiers (such as
//! `CUST-10291`) are stored as properties, never as MindGraph IDs.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::DomainError;

const UUID_V7: usize = 7;

fn validate_uuid(kind: &'static str, uuid: Uuid) -> Result<Uuid, DomainError> {
    if uuid.is_nil() {
        return Err(DomainError::InvalidId {
            kind,
            reason: "must not be the nil UUID".to_owned(),
        });
    }
    let version = uuid.get_version_num();
    if version != UUID_V7 {
        return Err(DomainError::InvalidId {
            kind,
            reason: format!("must be a UUIDv7, got version {version}"),
        });
    }
    Ok(uuid)
}

fn parse_uuid(kind: &'static str, value: &str) -> Result<Uuid, DomainError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(DomainError::InvalidId {
            kind,
            reason: "must not be empty".to_owned(),
        });
    }
    let uuid = Uuid::parse_str(value).map_err(|err| DomainError::InvalidId {
        kind,
        reason: err.to_string(),
    })?;
    validate_uuid(kind, uuid)
}

/// Defines one identifier newtype. A macro is used only because the
/// identifier types are otherwise identical; each still gets its own docs.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        ///
        /// Always a non-nil UUIDv7. Serialises as a hyphenated UUID string.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a new, time-ordered UUIDv7 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Wraps an existing UUID, for example one loaded from storage.
            ///
            /// # Errors
            ///
            /// Returns [`DomainError::InvalidId`] if the UUID is nil or is not
            /// version 7.
            pub fn from_uuid(uuid: Uuid) -> Result<Self, DomainError> {
                validate_uuid($kind, uuid).map(Self)
            }

            /// Returns the underlying UUID.
            #[must_use]
            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            /// Generates a new identifier, exactly like [`Self::new`].
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                parse_uuid($kind, value).map(Self)
            }
        }

        impl TryFrom<Uuid> for $name {
            type Error = DomainError;

            fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
                Self::from_uuid(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

define_id!(
    /// Identifies a [`Workspace`](crate::Workspace).
    WorkspaceId,
    "workspace id"
);

define_id!(
    /// Identifies a [`GraphProject`](crate::GraphProject), the logical graph
    /// that entities, relationships and ontologies belong to.
    GraphId,
    "graph id"
);

define_id!(
    /// Identifies an [`EntityType`](crate::EntityType).
    EntityTypeId,
    "entity type id"
);

define_id!(
    /// Identifies a [`RelationshipType`](crate::RelationshipType).
    RelationshipTypeId,
    "relationship type id"
);

define_id!(
    /// Identifies an [`Entity`](crate::Entity).
    EntityId,
    "entity id"
);

define_id!(
    /// Identifies a [`Relationship`](crate::Relationship).
    RelationshipId,
    "relationship id"
);

define_id!(
    /// Identifies a [`DataSource`](crate::DataSource): one uploaded dataset file.
    DataSourceId,
    "data source id"
);

define_id!(
    /// Identifies an ingestion or analysis job.
    IngestionJobId,
    "ingestion job id"
);

define_id!(
    /// Identifies one entity resolution scan (M9): a bounded duplicate
    /// candidate search over one entity type of one graph.
    ResolutionScanId,
    "resolution scan id"
);

define_id!(
    /// Identifies one duplicate candidate (a pair of entities) found by a
    /// resolution scan.
    ResolutionCandidateId,
    "resolution candidate id"
);

define_id!(
    /// Identifies one human review decision about a duplicate candidate.
    ResolutionDecisionId,
    "resolution decision id"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_are_version_7_and_not_nil() {
        let id = GraphId::new();
        assert_eq!(id.as_uuid().get_version_num(), 7);
        assert!(!id.as_uuid().is_nil());
    }

    #[test]
    fn new_ids_are_unique_and_time_ordered() {
        let ids: Vec<EntityId> = (0..1_000).map(|_| EntityId::new()).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn display_and_parse_round_trip() {
        let id = RelationshipId::new();
        let parsed: RelationshipId = id.to_string().parse().unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn parse_accepts_surrounding_whitespace() {
        let id = WorkspaceId::new();
        let parsed: WorkspaceId = format!("  {id}\n").parse().unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn parse_rejects_empty_input() {
        let err = "  ".parse::<EntityId>().unwrap_err();
        assert_eq!(
            err,
            DomainError::InvalidId {
                kind: "entity id",
                reason: "must not be empty".to_owned()
            }
        );
    }

    #[test]
    fn parse_rejects_malformed_input() {
        let err = "CUST-10291".parse::<EntityId>().unwrap_err();
        assert!(matches!(
            err,
            DomainError::InvalidId {
                kind: "entity id",
                ..
            }
        ));
    }

    #[test]
    fn nil_uuid_is_rejected() {
        let err = EntityId::from_uuid(Uuid::nil()).unwrap_err();
        assert_eq!(
            err,
            DomainError::InvalidId {
                kind: "entity id",
                reason: "must not be the nil UUID".to_owned()
            }
        );
        assert!(
            "00000000-0000-0000-0000-000000000000"
                .parse::<GraphId>()
                .is_err()
        );
    }

    #[test]
    fn non_v7_uuid_is_rejected() {
        // A valid UUIDv4.
        let v4 = Uuid::parse_str("9b2f4c1e-3a5d-4e8f-9c7b-1d2e3f4a5b6c").unwrap();
        let err = GraphId::from_uuid(v4).unwrap_err();
        assert_eq!(
            err,
            DomainError::InvalidId {
                kind: "graph id",
                reason: "must be a UUIDv7, got version 4".to_owned()
            }
        );
    }

    #[test]
    fn uuid_conversions_round_trip() {
        let id = EntityTypeId::new();
        let uuid: Uuid = id.into();
        assert_eq!(EntityTypeId::try_from(uuid).unwrap(), id);
    }

    #[test]
    fn serde_uses_plain_uuid_strings() {
        let id = GraphId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        let back: GraphId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn serde_rejects_invalid_ids() {
        let nil = "\"00000000-0000-0000-0000-000000000000\"";
        assert!(serde_json::from_str::<GraphId>(nil).is_err());
        assert!(serde_json::from_str::<GraphId>("\"\"").is_err());
    }
}
