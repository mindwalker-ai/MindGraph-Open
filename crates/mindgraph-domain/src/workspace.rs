//! Workspaces (ADR-003).

use chrono::{DateTime, Utc};

use crate::text::{MAX_DISPLAY_TEXT_CHARS, required_text};
use crate::{DomainError, WorkspaceId};

/// The top-level container that owns graph projects.
///
/// v0.1 runs with a single local workspace. Graphs still record their
/// workspace so multi-tenancy can be added later without remodelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    id: WorkspaceId,
    name: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Workspace {
    /// Creates a new workspace stamped with the current time.
    ///
    /// The name is trimmed and must be 1 to 120 characters long.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::EmptyValue`] or [`DomainError::TooLong`] for an
    /// invalid name.
    pub fn new(id: WorkspaceId, name: &str) -> Result<Self, DomainError> {
        let now = Utc::now();
        Self::from_parts(id, name, now, now)
    }

    /// Rebuilds a workspace from stored values, re-checking every invariant.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid name, or
    /// [`DomainError::InvalidTimestamps`] if `updated_at` is before
    /// `created_at`.
    pub fn from_parts(
        id: WorkspaceId,
        name: &str,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = required_text("workspace name", name, MAX_DISPLAY_TEXT_CHARS)?;
        if updated_at < created_at {
            return Err(DomainError::InvalidTimestamps);
        }
        Ok(Self {
            id,
            name,
            created_at,
            updated_at,
        })
    }

    /// The workspace identifier.
    #[must_use]
    pub const fn id(&self) -> WorkspaceId {
        self.id
    }

    /// The workspace name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// When the workspace was created.
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// When the workspace was last modified.
    #[must_use]
    pub const fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn new_workspace_has_trimmed_name_and_equal_timestamps() {
        let workspace = Workspace::new(WorkspaceId::new(), "  Local  ").unwrap();
        assert_eq!(workspace.name(), "Local");
        assert_eq!(workspace.created_at(), workspace.updated_at());
    }

    #[test]
    fn empty_name_is_rejected() {
        assert_eq!(
            Workspace::new(WorkspaceId::new(), " "),
            Err(DomainError::EmptyValue {
                field: "workspace name"
            })
        );
    }

    #[test]
    fn from_parts_rejects_updated_before_created() {
        let created = Utc::now();
        let updated = created - Duration::seconds(1);
        assert_eq!(
            Workspace::from_parts(WorkspaceId::new(), "Local", created, updated),
            Err(DomainError::InvalidTimestamps)
        );
    }
}
