//! The metadata port: persistence of control-plane data (ADR-005).
//!
//! Workspaces, graph projects, ontology versions, data sources and jobs live
//! here. Graph topology does not: entities and relationships belong to the
//! `GraphStore`, and source-file bytes belong to the `SourceStore`.

use async_trait::async_trait;
use mindgraph_domain::{
    DataSource, DataSourceId, GraphId, GraphProject, IngestionJobId, Ontology, OntologyVersion,
    Workspace, WorkspaceId,
};
use mindgraph_ingestion::{DatasetProfile, IngestionSummary};

use crate::{IngestionJob, JobError};

/// The name given to the local workspace when it is first created.
pub const LOCAL_WORKSPACE_NAME: &str = "Local";

/// An error from a [`MetadataStore`].
///
/// Backend-specific errors (for example database driver errors) never appear
/// in this type. They are kept as the [`source`](std::error::Error::source)
/// of the backend variants.
#[derive(Debug, thiserror::Error)]
pub enum MetadataStoreError {
    /// A referenced record does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The write would duplicate an existing record, or a state transition
    /// is not allowed from the record's current state.
    #[error("conflict: {0}")]
    Conflict(String),

    /// The request is invalid, for example an ontology for another graph.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The backend could not be reached.
    #[error("metadata store unavailable: {message}")]
    BackendUnavailable {
        /// What failed.
        message: String,
        /// The underlying error, kept for diagnostics.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
    },

    /// The backend rejected the request or failed unexpectedly.
    #[error("metadata store error: {message}")]
    BackendError {
        /// What failed.
        message: String,
        /// The underlying error, kept for diagnostics.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
    },

    /// Stored data cannot be turned back into domain objects.
    #[error("corrupt metadata: {0}")]
    CorruptData(String),

    /// Domain data could not be encoded for storage.
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// A data source together with its private storage metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct DataSourceRecord {
    /// The public data source.
    pub data_source: DataSource,
    /// The backend-relative key of the source file. Never shown to clients.
    pub storage_key: String,
    /// The profile, once an analysis job has succeeded.
    pub profile: Option<DatasetProfile>,
}

/// Persistence of workspaces, graph projects, ontology versions, data
/// sources and jobs.
///
/// Object safe and `Send + Sync`, so it can be shared as
/// `Arc<dyn MetadataStore>`.
///
/// # Contract
///
/// - Records are never overwritten by create operations. A duplicate is a
///   [`Conflict`](MetadataStoreError::Conflict).
/// - Ontology versions are immutable. New versions are added with
///   [`create_ontology`](Self::create_ontology) or
///   [`create_and_activate_ontology`](Self::create_and_activate_ontology).
/// - A graph project always has an active ontology version that exists.
/// - Data sources and jobs are scoped to their graph: a data source of graph
///   A is not found through graph B.
/// - Jobs move only `queued → running → completed | failed`, or
///   `queued → failed`. Other transitions are a
///   [`Conflict`](MetadataStoreError::Conflict). Progress never decreases
///   and stays within `0.0..=1.0`.
/// - Timestamps are stored with microsecond precision. Callers that need
///   identical values after a round trip should truncate to microseconds.
#[async_trait]
pub trait MetadataStore: Send + Sync {
    /// Returns the single local workspace, creating it on first use.
    /// Idempotent: every call, across restarts, returns the same workspace.
    async fn ensure_local_workspace(&self) -> Result<Workspace, MetadataStoreError>;

    /// Reads a workspace.
    async fn get_workspace(&self, id: WorkspaceId)
    -> Result<Option<Workspace>, MetadataStoreError>;

    /// Atomically stores a new graph project, its initial ontology, and
    /// marks that ontology as active. Either everything is stored or nothing.
    ///
    /// # Errors
    ///
    /// [`Conflict`](MetadataStoreError::Conflict) if the graph exists,
    /// [`NotFound`](MetadataStoreError::NotFound) if its workspace does not,
    /// and [`InvalidInput`](MetadataStoreError::InvalidInput) if the ontology
    /// belongs to another graph.
    async fn create_graph(
        &self,
        graph: &GraphProject,
        initial_ontology: &Ontology,
    ) -> Result<(), MetadataStoreError>;

    /// Reads a graph project.
    async fn get_graph(&self, id: GraphId) -> Result<Option<GraphProject>, MetadataStoreError>;

    /// Lists the graph projects of a workspace, newest first (`created_at`
    /// descending, then id descending), at most `limit` of them.
    async fn list_graphs(
        &self,
        workspace_id: WorkspaceId,
        limit: usize,
    ) -> Result<Vec<GraphProject>, MetadataStoreError>;

    /// Moves a `draft` graph to `active` and returns it. An `active` graph is
    /// returned unchanged; an `archived` graph is not reactivated.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) if the graph does not exist.
    async fn activate_graph(&self, id: GraphId) -> Result<GraphProject, MetadataStoreError>;

    /// Stores a new ontology version for an existing graph. Does not change
    /// which version is active.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) if the graph does not exist,
    /// [`Conflict`](MetadataStoreError::Conflict) if the version exists.
    async fn create_ontology(&self, ontology: &Ontology) -> Result<(), MetadataStoreError>;

    /// Atomically stores a new ontology version and makes it the active one.
    ///
    /// # Errors
    ///
    /// The errors of [`create_ontology`](Self::create_ontology). Nothing is
    /// stored on error.
    async fn create_and_activate_ontology(
        &self,
        ontology: &Ontology,
    ) -> Result<(), MetadataStoreError>;

    /// Reads one ontology version.
    async fn get_ontology(
        &self,
        graph_id: GraphId,
        version: OntologyVersion,
    ) -> Result<Option<Ontology>, MetadataStoreError>;

    /// Reads the active ontology of a graph, or `None` if the graph does not
    /// exist.
    async fn get_active_ontology(
        &self,
        graph_id: GraphId,
    ) -> Result<Option<Ontology>, MetadataStoreError>;

    /// Makes an existing ontology version the active one.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) if the graph or the version
    /// does not exist.
    async fn set_active_ontology(
        &self,
        graph_id: GraphId,
        version: OntologyVersion,
    ) -> Result<(), MetadataStoreError>;

    /// Stores a new (pending) data source.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) if the graph does not
    /// exist, [`Conflict`](MetadataStoreError::Conflict) if the id exists.
    async fn create_data_source(
        &self,
        data_source: &DataSource,
        storage_key: &str,
    ) -> Result<(), MetadataStoreError>;

    /// Reads a data source of a graph. A data source of another graph is
    /// `None`.
    async fn get_data_source(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<Option<DataSourceRecord>, MetadataStoreError>;

    /// Stores the profile of a data source and marks it `ready`.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) if it does not exist in the
    /// graph.
    async fn mark_data_source_ready(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
        profile: &DatasetProfile,
    ) -> Result<DataSource, MetadataStoreError>;

    /// Marks a data source `failed` and removes any stored profile.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) if it does not exist in the
    /// graph.
    async fn mark_data_source_failed(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<DataSource, MetadataStoreError>;

    /// Stores a new job.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) if its data source does not
    /// exist in its graph, [`Conflict`](MetadataStoreError::Conflict) if the
    /// id or the graph's idempotency key is already used, and
    /// [`InvalidInput`](MetadataStoreError::InvalidInput) for a pinned
    /// ontology version that does not exist or a job that is not `queued`.
    async fn create_job(&self, job: &IngestionJob) -> Result<(), MetadataStoreError>;

    /// Reads a job.
    async fn get_job(&self, id: IngestionJobId)
    -> Result<Option<IngestionJob>, MetadataStoreError>;

    /// Finds the job a graph created with an idempotency key.
    async fn find_job_by_idempotency_key(
        &self,
        graph_id: GraphId,
        key: &str,
    ) -> Result<Option<IngestionJob>, MetadataStoreError>;

    /// `queued → running`. Returns the updated job.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) or
    /// [`Conflict`](MetadataStoreError::Conflict) if the job is not queued.
    async fn start_job(&self, id: IngestionJobId) -> Result<IngestionJob, MetadataStoreError>;

    /// Raises the progress of a running job. Values are clamped to
    /// `0.0..=1.0`; a value below the stored progress is ignored.
    ///
    /// # Errors
    ///
    /// [`InvalidInput`](MetadataStoreError::InvalidInput) for NaN,
    /// [`NotFound`](MetadataStoreError::NotFound), or
    /// [`Conflict`](MetadataStoreError::Conflict) if the job is not running.
    async fn update_job_progress(
        &self,
        id: IngestionJobId,
        progress: f64,
    ) -> Result<(), MetadataStoreError>;

    /// `running → completed`, with progress `1.0` and an optional result.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) or
    /// [`Conflict`](MetadataStoreError::Conflict) if the job is not running.
    async fn complete_job(
        &self,
        id: IngestionJobId,
        result: Option<&IngestionSummary>,
    ) -> Result<IngestionJob, MetadataStoreError>;

    /// `queued | running → failed`, recording the error. Progress is kept.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) or
    /// [`Conflict`](MetadataStoreError::Conflict) if the job already finished.
    async fn fail_job(
        &self,
        id: IngestionJobId,
        error: &JobError,
    ) -> Result<IngestionJob, MetadataStoreError>;

    /// Fails every `queued` or `running` job (restart recovery) and returns
    /// how many were changed.
    async fn mark_incomplete_jobs_failed(
        &self,
        error: &JobError,
    ) -> Result<u64, MetadataStoreError>;

    /// Checks that the backend is reachable (used for readiness).
    async fn ping(&self) -> Result<(), MetadataStoreError>;
}
