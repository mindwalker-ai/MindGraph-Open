//! Data sources: uploaded dataset files (ADR-008).
//!
//! A data source records *what* was uploaded (kind, original filename, size,
//! checksum, status). Where the bytes live is an infrastructure concern and
//! is deliberately not part of this type.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};

use crate::text::required_text;
use crate::{DataSourceId, DomainError, GraphId};

/// Longest accepted original filename.
const MAX_FILENAME_CHARS: usize = 255;

/// The format of an uploaded dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataSourceKind {
    /// Comma-separated values with a header row.
    Csv,
    /// A top-level JSON array of flat objects.
    Json,
}

impl DataSourceKind {
    /// The canonical lowercase name (`csv`, `json`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }

    /// The kind implied by a filename's extension (case-insensitive), if
    /// supported.
    #[must_use]
    pub fn from_filename(filename: &str) -> Option<Self> {
        let (_, extension) = filename.rsplit_once('.')?;
        match extension.to_ascii_lowercase().as_str() {
            "csv" => Some(Self::Csv),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

impl fmt::Display for DataSourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DataSourceKind {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "csv" => Ok(Self::Csv),
            "json" => Ok(Self::Json),
            other => Err(unknown("data source kind", other)),
        }
    }
}

/// The lifecycle status of a data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataSourceStatus {
    /// Uploaded, not analysed yet.
    Pending,
    /// Analysed successfully; its profile is available.
    Ready,
    /// Analysis failed.
    Failed,
}

impl DataSourceStatus {
    /// The canonical lowercase name (`pending`, `ready`, `failed`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for DataSourceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DataSourceStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "ready" => Ok(Self::Ready),
            "failed" => Ok(Self::Failed),
            other => Err(unknown("data source status", other)),
        }
    }
}

fn unknown(field: &'static str, value: &str) -> DomainError {
    DomainError::InvalidName {
        field,
        value: value.to_owned(),
        reason: "unknown value",
    }
}

/// One uploaded dataset file belonging to a graph project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSource {
    id: DataSourceId,
    graph_id: GraphId,
    kind: DataSourceKind,
    filename: String,
    status: DataSourceStatus,
    size_bytes: u64,
    content_sha256: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl DataSource {
    /// A newly uploaded data source in [`DataSourceStatus::Pending`].
    ///
    /// # Errors
    ///
    /// See [`Self::from_parts`].
    pub fn new(
        id: DataSourceId,
        graph_id: GraphId,
        kind: DataSourceKind,
        filename: &str,
        size_bytes: u64,
        content_sha256: &str,
        created_at: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        Self::from_parts(
            id,
            graph_id,
            kind,
            filename,
            DataSourceStatus::Pending,
            size_bytes,
            content_sha256,
            created_at,
            created_at,
        )
    }

    /// Checks an original filename and returns it trimmed: 1–255
    /// characters, no path separators and no control characters. The
    /// filename is metadata only; it is never used as a filesystem path.
    ///
    /// # Errors
    ///
    /// [`DomainError::EmptyValue`], [`DomainError::TooLong`] or
    /// [`DomainError::InvalidName`].
    pub fn validate_filename(filename: &str) -> Result<String, DomainError> {
        let filename = required_text("filename", filename, MAX_FILENAME_CHARS)?;
        if filename
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
        {
            return Err(DomainError::InvalidName {
                field: "filename",
                value: filename,
                reason: "must not contain path separators or control characters",
            });
        }
        Ok(filename)
    }

    /// Rebuilds a data source from stored values, re-checking every invariant.
    ///
    /// The filename is metadata only: 1–255 characters, no path separators
    /// and no control characters. The checksum is 64 lowercase hex digits.
    ///
    /// # Errors
    ///
    /// [`DomainError::EmptyValue`], [`DomainError::TooLong`] or
    /// [`DomainError::InvalidName`] for an invalid filename or checksum, and
    /// [`DomainError::InvalidTimestamps`] if `updated_at < created_at`.
    #[allow(clippy::too_many_arguments)] // A flat record; a builder would add no clarity.
    pub fn from_parts(
        id: DataSourceId,
        graph_id: GraphId,
        kind: DataSourceKind,
        filename: &str,
        status: DataSourceStatus,
        size_bytes: u64,
        content_sha256: &str,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let filename = Self::validate_filename(filename)?;
        let checksum_ok = content_sha256.len() == 64
            && content_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !checksum_ok {
            return Err(DomainError::InvalidName {
                field: "content_sha256",
                value: content_sha256.to_owned(),
                reason: "must be 64 lowercase hexadecimal digits",
            });
        }
        if updated_at < created_at {
            return Err(DomainError::InvalidTimestamps);
        }
        Ok(Self {
            id,
            graph_id,
            kind,
            filename,
            status,
            size_bytes,
            content_sha256: content_sha256.to_owned(),
            created_at,
            updated_at,
        })
    }

    /// The data source identifier.
    #[must_use]
    pub const fn id(&self) -> DataSourceId {
        self.id
    }

    /// The graph project this data source belongs to.
    #[must_use]
    pub const fn graph_id(&self) -> GraphId {
        self.graph_id
    }

    /// The dataset format.
    #[must_use]
    pub const fn kind(&self) -> DataSourceKind {
        self.kind
    }

    /// The original filename (metadata only, never a filesystem path).
    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// The lifecycle status.
    #[must_use]
    pub const fn status(&self) -> DataSourceStatus {
        self.status
    }

    /// The file size in bytes.
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// The SHA-256 of the file contents, lowercase hex.
    #[must_use]
    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }

    /// When the data source was uploaded.
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// When the data source last changed.
    #[must_use]
    pub const fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn source(filename: &str, sha: &str) -> Result<DataSource, DomainError> {
        DataSource::new(
            DataSourceId::new(),
            GraphId::new(),
            DataSourceKind::Csv,
            filename,
            42,
            sha,
            Utc::now(),
        )
    }

    #[test]
    fn new_data_sources_are_pending() {
        let source = source("customers.csv", SHA).unwrap();
        assert_eq!(source.status(), DataSourceStatus::Pending);
        assert_eq!(source.filename(), "customers.csv");
        assert_eq!(source.size_bytes(), 42);
    }

    #[test]
    fn kinds_come_from_extensions() {
        assert_eq!(
            DataSourceKind::from_filename("a.CSV"),
            Some(DataSourceKind::Csv)
        );
        assert_eq!(
            DataSourceKind::from_filename("x.y.json"),
            Some(DataSourceKind::Json)
        );
        assert_eq!(DataSourceKind::from_filename("notes.txt"), None);
        assert_eq!(DataSourceKind::from_filename("csv"), None);
    }

    #[test]
    fn filenames_are_metadata_only() {
        for bad in [
            "",
            "  ",
            "../etc/passwd",
            "a/b.csv",
            "a\\b.csv",
            "bad\nname.csv",
        ] {
            assert!(source(bad, SHA).is_err(), "{bad:?}");
        }
        assert!(source(&"a".repeat(256), SHA).is_err());
    }

    #[test]
    fn checksums_are_validated() {
        assert!(source("a.csv", "abc").is_err());
        assert!(source("a.csv", &SHA.to_uppercase()).is_err());
    }

    #[test]
    fn kinds_and_statuses_round_trip() {
        for kind in [DataSourceKind::Csv, DataSourceKind::Json] {
            assert_eq!(kind.as_str().parse::<DataSourceKind>().unwrap(), kind);
        }
        for status in [
            DataSourceStatus::Pending,
            DataSourceStatus::Ready,
            DataSourceStatus::Failed,
        ] {
            assert_eq!(status.as_str().parse::<DataSourceStatus>().unwrap(), status);
        }
        assert!("xml".parse::<DataSourceKind>().is_err());
    }
}
