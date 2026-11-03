//! Graph projects (ADR-003).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::text::{MAX_DESCRIPTION_CHARS, MAX_DISPLAY_TEXT_CHARS, optional_text, required_text};
use crate::{DomainError, GraphId, WorkspaceId};

/// The lifecycle status of a graph project.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphStatus {
    /// Created, but no data has been loaded yet.
    #[default]
    Draft,
    /// Contains data and can be explored.
    Active,
    /// Hidden from default listings. Its data is retained.
    Archived,
}

impl GraphStatus {
    /// Returns the canonical lowercase name of the status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }
}

impl fmt::Display for GraphStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GraphStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            other => Err(DomainError::UnknownGraphStatus(other.to_owned())),
        }
    }
}

/// One logical graph, such as "Fraud Investigation" or "Company Knowledge
/// Graph".
///
/// Entities, relationships and ontology versions all belong to exactly one
/// graph project, identified by its [`GraphId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphProject {
    id: GraphId,
    workspace_id: WorkspaceId,
    name: String,
    description: Option<String>,
    status: GraphStatus,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl GraphProject {
    /// Creates a new graph project in [`GraphStatus::Draft`], stamped with
    /// the current time.
    ///
    /// The name is trimmed and must be 1 to 120 characters long. The
    /// description is optional, trimmed, and at most 2,000 characters. A
    /// blank description is stored as `None`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::EmptyValue`] or [`DomainError::TooLong`].
    pub fn new(
        id: GraphId,
        workspace_id: WorkspaceId,
        name: &str,
        description: Option<&str>,
    ) -> Result<Self, DomainError> {
        let now = Utc::now();
        Self::from_parts(
            id,
            workspace_id,
            name,
            description,
            GraphStatus::Draft,
            now,
            now,
        )
    }

    /// Rebuilds a graph project from stored values, re-checking every
    /// invariant.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid name or description, or
    /// [`DomainError::InvalidTimestamps`] if `updated_at` is before
    /// `created_at`.
    pub fn from_parts(
        id: GraphId,
        workspace_id: WorkspaceId,
        name: &str,
        description: Option<&str>,
        status: GraphStatus,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = required_text("graph name", name, MAX_DISPLAY_TEXT_CHARS)?;
        let description = optional_text("graph description", description, MAX_DESCRIPTION_CHARS)?;
        if updated_at < created_at {
            return Err(DomainError::InvalidTimestamps);
        }
        Ok(Self {
            id,
            workspace_id,
            name,
            description,
            status,
            created_at,
            updated_at,
        })
    }

    /// The graph identifier.
    #[must_use]
    pub const fn id(&self) -> GraphId {
        self.id
    }

    /// The workspace that owns this graph.
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    /// The graph name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The optional description.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The lifecycle status.
    #[must_use]
    pub const fn status(&self) -> GraphStatus {
        self.status
    }

    /// When the graph was created.
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// When the graph was last modified.
    #[must_use]
    pub const fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn project(name: &str, description: Option<&str>) -> Result<GraphProject, DomainError> {
        GraphProject::new(GraphId::new(), WorkspaceId::new(), name, description)
    }

    #[test]
    fn new_project_is_a_draft_with_trimmed_fields() {
        let graph = project("  Fraud Investigation ", Some("  Card fraud ring  ")).unwrap();
        assert_eq!(graph.name(), "Fraud Investigation");
        assert_eq!(graph.description(), Some("Card fraud ring"));
        assert_eq!(graph.status(), GraphStatus::Draft);
        assert_eq!(graph.created_at(), graph.updated_at());
    }

    #[test]
    fn empty_graph_names_are_invalid() {
        for name in ["", "   ", "\n\t"] {
            assert_eq!(
                project(name, None),
                Err(DomainError::EmptyValue {
                    field: "graph name"
                })
            );
        }
    }

    #[test]
    fn overly_long_graph_names_are_invalid() {
        assert!(project(&"g".repeat(120), None).is_ok());
        assert_eq!(
            project(&"g".repeat(121), None),
            Err(DomainError::TooLong {
                field: "graph name",
                max: 120
            })
        );
    }

    #[test]
    fn blank_descriptions_become_none() {
        assert_eq!(project("Cyber", Some("   ")).unwrap().description(), None);
        assert_eq!(project("Cyber", None).unwrap().description(), None);
    }

    #[test]
    fn overly_long_descriptions_are_invalid() {
        assert_eq!(
            project("Cyber", Some(&"d".repeat(2_001))),
            Err(DomainError::TooLong {
                field: "graph description",
                max: 2_000
            })
        );
    }

    #[test]
    fn from_parts_preserves_stored_values() {
        let id = GraphId::new();
        let workspace_id = WorkspaceId::new();
        let created = Utc::now() - Duration::days(1);
        let updated = Utc::now();
        let graph = GraphProject::from_parts(
            id,
            workspace_id,
            "Knowledge",
            None,
            GraphStatus::Active,
            created,
            updated,
        )
        .unwrap();
        assert_eq!(graph.id(), id);
        assert_eq!(graph.workspace_id(), workspace_id);
        assert_eq!(graph.status(), GraphStatus::Active);
        assert_eq!(graph.created_at(), created);
        assert_eq!(graph.updated_at(), updated);
    }

    #[test]
    fn from_parts_rejects_updated_before_created() {
        let created = Utc::now();
        let result = GraphProject::from_parts(
            GraphId::new(),
            WorkspaceId::new(),
            "Knowledge",
            None,
            GraphStatus::Draft,
            created,
            created - Duration::milliseconds(1),
        );
        assert_eq!(result, Err(DomainError::InvalidTimestamps));
    }

    #[test]
    fn status_round_trips_through_strings_and_serde() {
        for status in [
            GraphStatus::Draft,
            GraphStatus::Active,
            GraphStatus::Archived,
        ] {
            assert_eq!(status.as_str().parse::<GraphStatus>().unwrap(), status);
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!("\"{status}\""));
        }
        assert_eq!(
            "Deleted".parse::<GraphStatus>(),
            Err(DomainError::UnknownGraphStatus("Deleted".to_owned()))
        );
    }
}
