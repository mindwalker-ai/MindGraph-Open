//! Background job records: dataset analysis, ingestion and AI Graph
//! Builder suggestions.
//!
//! Job state is persisted through the [`MetadataStore`](crate::MetadataStore)
//! and moves only along controlled transitions:
//!
//! ```text
//! queued ──► running ──► completed
//!    │          └──────► failed
//!    └─────────────────► failed      (restart recovery, queue rejection)
//! ```
//!
//! `cancelled` exists in the schema, but no job can be cancelled yet (see the
//! limitations in the architecture notes).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use mindgraph_domain::{DataSourceId, DomainError, GraphId, IngestionJobId, OntologyVersion};
use mindgraph_ingestion::{IngestionMapping, IngestionSummary};

use crate::GraphBuilderResult;

/// Largest number of data sources one AI suggestion may model together.
pub const MAX_SUGGESTION_DATA_SOURCES: usize = 10;

/// What a job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// Profiles a data source.
    Analyze,
    /// Writes a data source into the graph.
    Ingest,
    /// Asks the AI Graph Builder for an ontology and mapping suggestion.
    AiSuggest,
}

impl JobKind {
    /// The canonical lowercase name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Analyze => "analyze",
            Self::Ingest => "ingest",
            Self::AiSuggest => "ai_suggest",
        }
    }
}

impl fmt::Display for JobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for JobKind {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "analyze" => Ok(Self::Analyze),
            "ingest" => Ok(Self::Ingest),
            "ai_suggest" => Ok(Self::AiSuggest),
            other => Err(unknown("job kind", other)),
        }
    }
}

/// The lifecycle state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobStatus {
    /// Accepted, waiting for a worker.
    Queued,
    /// A worker is executing it.
    Running,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Stopped on request (not produced in M4).
    Cancelled,
}

impl JobStatus {
    /// The canonical lowercase name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether the job can no longer change.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for JobStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(unknown("job status", other)),
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

/// A bounded, client-safe description of why a job failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobError {
    /// A stable code, for example `TYPE_CONVERSION` or `PROCESS_RESTARTED`.
    pub code: String,
    /// A human-readable message without record values.
    pub message: String,
}

/// The longest stored error message, in characters.
pub const MAX_JOB_ERROR_CHARS: usize = 1_000;

impl JobError {
    /// Creates an error, truncating the message to [`MAX_JOB_ERROR_CHARS`].
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl AsRef<str>) -> Self {
        Self {
            code: code.into(),
            message: message.as_ref().chars().take(MAX_JOB_ERROR_CHARS).collect(),
        }
    }
}

/// What a completed job produced. Analysis jobs have no result: the
/// profile is stored on the data source.
#[derive(Debug, Clone, PartialEq)]
pub enum JobResult {
    /// Counters of an ingestion job.
    Ingestion(IngestionSummary),
    /// The validated suggestion of an AI Graph Builder job.
    GraphBuilder(Box<GraphBuilderResult>),
}

impl JobResult {
    /// The job kind this result belongs to.
    #[must_use]
    pub const fn kind(&self) -> JobKind {
        match self {
            Self::Ingestion(_) => JobKind::Ingest,
            Self::GraphBuilder(_) => JobKind::AiSuggest,
        }
    }
}

/// A persisted analysis, ingestion or AI suggestion job.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestionJob {
    /// The job id.
    pub id: IngestionJobId,
    /// The graph the job belongs to.
    pub graph_id: GraphId,
    /// The data source it reads; for an AI suggestion over several data
    /// sources, the first of [`Self::data_source_ids`].
    pub data_source_id: DataSourceId,
    /// Every data source the job reads, in order: `[data_source_id]` for
    /// analysis and ingestion, `1..=MAX_SUGGESTION_DATA_SOURCES` distinct
    /// data sources for an AI suggestion.
    pub data_source_ids: Vec<DataSourceId>,
    /// Analysis, ingestion or AI suggestion.
    pub kind: JobKind,
    /// The lifecycle state.
    pub status: JobStatus,
    /// `0.0..=1.0`; never decreases; `1.0` once completed.
    pub progress: f64,
    /// The ontology version the job is pinned to: the version an ingestion
    /// was validated against, or the base of an AI suggestion. Set for
    /// [`JobKind::Ingest`] and [`JobKind::AiSuggest`], never for
    /// [`JobKind::Analyze`].
    pub ontology_version: Option<OntologyVersion>,
    /// The client's `Idempotency-Key`, unique per graph.
    pub idempotency_key: Option<String>,
    /// The mapping of an ingestion job.
    pub mapping: Option<IngestionMapping>,
    /// Why the job failed.
    pub error: Option<JobError>,
    /// What a completed ingestion or AI suggestion produced.
    pub result: Option<JobResult>,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// When the job last changed.
    pub updated_at: DateTime<Utc>,
}

impl IngestionJob {
    /// A queued analysis job.
    #[must_use]
    pub fn analysis(graph_id: GraphId, data_source_id: DataSourceId, now: DateTime<Utc>) -> Self {
        Self {
            id: IngestionJobId::new(),
            graph_id,
            data_source_id,
            data_source_ids: vec![data_source_id],
            kind: JobKind::Analyze,
            status: JobStatus::Queued,
            progress: 0.0,
            ontology_version: None,
            idempotency_key: None,
            mapping: None,
            error: None,
            result: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// A queued ingestion job pinned to `ontology_version`.
    #[must_use]
    pub fn ingestion(
        graph_id: GraphId,
        data_source_id: DataSourceId,
        ontology_version: OntologyVersion,
        idempotency_key: String,
        mapping: IngestionMapping,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            kind: JobKind::Ingest,
            ontology_version: Some(ontology_version),
            idempotency_key: Some(idempotency_key),
            mapping: Some(mapping),
            ..Self::analysis(graph_id, data_source_id, now)
        }
    }

    /// A queued AI Graph Builder job over `data_source_ids` whose suggestion
    /// builds on `ontology_version` (pinned).
    ///
    /// # Panics
    ///
    /// If `data_source_ids` is empty; [`Self::validate_new`] checks the
    /// other bounds.
    #[must_use]
    pub fn graph_builder(
        graph_id: GraphId,
        data_source_ids: Vec<DataSourceId>,
        ontology_version: OntologyVersion,
        idempotency_key: String,
        now: DateTime<Utc>,
    ) -> Self {
        let first = *data_source_ids
            .first()
            .expect("an AI suggestion reads at least one data source");
        Self {
            kind: JobKind::AiSuggest,
            data_source_ids,
            ontology_version: Some(ontology_version),
            idempotency_key: Some(idempotency_key),
            ..Self::analysis(graph_id, first, now)
        }
    }

    /// The counters of a completed ingestion job.
    #[must_use]
    pub fn ingestion_summary(&self) -> Option<&IngestionSummary> {
        match &self.result {
            Some(JobResult::Ingestion(summary)) => Some(summary),
            _ => None,
        }
    }

    /// The result of a completed AI Graph Builder job.
    #[must_use]
    pub fn graph_builder_result(&self) -> Option<&GraphBuilderResult> {
        match &self.result {
            Some(JobResult::GraphBuilder(result)) => Some(result),
            _ => None,
        }
    }

    /// Checks the invariants of a job about to be stored: queued, no
    /// progress, error or result; a pinned ontology version plus mapping for
    /// ingestion jobs; a pinned version and an idempotency key, but no
    /// mapping, for AI suggestion jobs. Metadata stores call this in
    /// `create_job`.
    ///
    /// # Errors
    ///
    /// A description of the first broken invariant.
    pub fn validate_new(&self) -> Result<(), String> {
        let invalid = |reason: &str| Err(format!("job {}: {reason}", self.id));
        if self.status != JobStatus::Queued
            || self.progress != 0.0
            || self.error.is_some()
            || self.result.is_some()
        {
            return invalid("new jobs must be queued with no progress, error or result");
        }
        if self.data_source_ids.first() != Some(&self.data_source_id) {
            return invalid("the first data source must be `data_source_id`");
        }
        let distinct: std::collections::HashSet<_> = self.data_source_ids.iter().collect();
        if distinct.len() != self.data_source_ids.len() {
            return invalid("data sources must be distinct");
        }
        let max_sources = if self.kind == JobKind::AiSuggest {
            MAX_SUGGESTION_DATA_SOURCES
        } else {
            1
        };
        if self.data_source_ids.len() > max_sources {
            return invalid("too many data sources for this job kind");
        }
        match self.kind {
            JobKind::Ingest if self.ontology_version.is_none() || self.mapping.is_none() => {
                invalid("ingestion jobs need a pinned ontology version and a mapping")
            }
            JobKind::Analyze if self.ontology_version.is_some() || self.mapping.is_some() => {
                invalid("analysis jobs have no ontology version or mapping")
            }
            JobKind::AiSuggest
                if self.ontology_version.is_none()
                    || self.mapping.is_some()
                    || self.idempotency_key.is_none() =>
            {
                invalid(
                    "AI suggestion jobs need a pinned ontology version and an idempotency key, \
                     and have no mapping",
                )
            }
            _ if self.idempotency_key.as_deref().is_some_and(str::is_empty) => {
                invalid("idempotency keys must not be empty")
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for kind in [JobKind::Analyze, JobKind::Ingest, JobKind::AiSuggest] {
            assert_eq!(kind.as_str().parse::<JobKind>().unwrap(), kind);
        }
        for status in [
            JobStatus::Queued,
            JobStatus::Running,
            JobStatus::Completed,
            JobStatus::Failed,
            JobStatus::Cancelled,
        ] {
            assert_eq!(status.as_str().parse::<JobStatus>().unwrap(), status);
        }
        assert!("paused".parse::<JobStatus>().is_err());
        assert!("export".parse::<JobKind>().is_err());
    }

    #[test]
    fn terminal_states() {
        assert!(!JobStatus::Queued.is_terminal());
        assert!(!JobStatus::Running.is_terminal());
        assert!(JobStatus::Completed.is_terminal());
        assert!(JobStatus::Failed.is_terminal());
        assert!(JobStatus::Cancelled.is_terminal());
    }

    #[test]
    fn error_messages_are_bounded() {
        let error = JobError::new("X", "é".repeat(5_000));
        assert_eq!(error.message.chars().count(), MAX_JOB_ERROR_CHARS);
    }
}
