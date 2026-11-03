//! Dataset use cases: upload, analysis jobs, ingestion jobs (M4).
//!
//! ```text
//! begin_upload ─► UploadSession::write_chunk* ─► finish ─► DataSource (pending)
//! start_analysis ─► job queued ─► worker: profile ─► DataSource ready
//! start_ingestion ─► validate mapping against the active ontology
//!                 ─► job queued (ontology version pinned)
//!                 ─► worker: preflight + batched GraphStore writes
//!                 ─► graph draft → active, job completed
//! ```
//!
//! Jobs run in-process on Tokio through the shared executor: a dispatcher
//! takes job ids from a bounded queue and runs at most `job_concurrency` of
//! them at a time (a semaphore), whatever their kind.
//! Job state is persisted in the [`MetadataStore`] after every transition,
//! so clients poll it (or follow it over SSE) and a restart can detect jobs
//! that were cut off ([`IngestionService::recover_incomplete_jobs`]).

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use mindgraph_domain::{
    DataSource, DataSourceId, DataSourceKind, DataSourceStatus, GraphId, IngestionJobId,
    WorkspaceId,
};
use mindgraph_graph::GraphStore;
use mindgraph_ingestion::{
    DatasetProfile, IngestionError, IngestionMapping, IngestionRun, IngestionSummary, Limits,
    ProgressSink, SourceStore, SourceWriter, UploadDigest, profile_dataset, run_ingestion,
    validate_mapping,
};
use tracing::{info, warn};

use crate::executor::{JobExecutor, JobHandler};
use crate::{
    ApplicationError, IngestionJob, JobError, JobKind, JobResult, MetadataStore, MetadataStoreError,
};

/// The default upload limit: 50 MiB.
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// The default number of records per graph write batch.
pub const DEFAULT_BATCH_SIZE: usize = 500;
/// The largest accepted batch size.
pub const MAX_BATCH_SIZE: usize = 5_000;
/// The default number of jobs running at the same time.
pub const DEFAULT_JOB_CONCURRENCY: usize = 2;
/// The default number of jobs waiting for a worker.
pub const DEFAULT_QUEUE_CAPACITY: usize = 256;
/// The longest accepted `Idempotency-Key`.
pub const MAX_IDEMPOTENCY_KEY_CHARS: usize = 255;
/// The error code of jobs cut off by a restart.
pub const PROCESS_RESTARTED: &str = "PROCESS_RESTARTED";

/// Bounds and tuning of dataset processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestionConfig {
    /// Largest accepted upload, in bytes.
    pub max_upload_bytes: u64,
    /// Record, field and value-size limits for analysis and ingestion.
    pub limits: Limits,
    /// Records per graph write batch (`1..=MAX_BATCH_SIZE`).
    pub batch_size: usize,
    /// Jobs running at the same time (at least 1).
    pub job_concurrency: usize,
    /// Jobs waiting for a worker; more are rejected as unavailable.
    pub queue_capacity: usize,
}

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
            limits: Limits::default(),
            batch_size: DEFAULT_BATCH_SIZE,
            job_concurrency: DEFAULT_JOB_CONCURRENCY,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
        }
    }
}

/// A data source as clients see it: no storage details.
#[derive(Debug, Clone, PartialEq)]
pub struct DataSourceView {
    /// The data source.
    pub data_source: DataSource,
    /// The profile, once analysed.
    pub profile: Option<DatasetProfile>,
}

/// Dataset upload, analysis and ingestion for the local workspace.
#[derive(Clone)]
pub struct IngestionService {
    runner: Arc<Runner>,
    executor: JobExecutor,
    workspace_id: WorkspaceId,
}

impl std::fmt::Debug for IngestionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IngestionService")
            .field("workspace_id", &self.workspace_id)
            .field("config", &self.runner.config)
            .finish_non_exhaustive()
    }
}

/// The worker of analysis and ingestion jobs.
struct Runner {
    metadata: Arc<dyn MetadataStore>,
    graph: Arc<dyn GraphStore>,
    sources: Arc<dyn SourceStore>,
    config: IngestionConfig,
    /// Ingestion jobs of one graph run one at a time. GraphStore writes are
    /// not transactional, so two concurrent jobs could both miss an identity
    /// and create it twice.
    graph_locks: std::sync::Mutex<HashMap<GraphId, Arc<tokio::sync::Mutex<()>>>>,
}

/// The ports the ingestion service uses.
pub struct IngestionPorts {
    /// Control-plane persistence.
    pub metadata: Arc<dyn MetadataStore>,
    /// Graph persistence.
    pub graph: Arc<dyn GraphStore>,
    /// Source-file persistence.
    pub sources: Arc<dyn SourceStore>,
}

impl IngestionService {
    /// Starts the job dispatcher and returns the service. Must be called
    /// inside a Tokio runtime.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for an out-of-range
    /// configuration value.
    pub fn start(
        ports: IngestionPorts,
        workspace_id: WorkspaceId,
        config: IngestionConfig,
    ) -> Result<Self, ApplicationError> {
        if !(1..=MAX_BATCH_SIZE).contains(&config.batch_size) {
            return Err(ApplicationError::InvalidRequest(format!(
                "the batch size must be between 1 and {MAX_BATCH_SIZE}"
            )));
        }
        if config.job_concurrency == 0 || config.queue_capacity == 0 || config.max_upload_bytes == 0
        {
            return Err(ApplicationError::InvalidRequest(
                "job concurrency, queue capacity and upload limit must be positive".to_owned(),
            ));
        }
        let executor = JobExecutor::start(
            Arc::clone(&ports.metadata),
            config.job_concurrency,
            config.queue_capacity,
        );
        let runner = Arc::new(Runner {
            metadata: ports.metadata,
            graph: ports.graph,
            sources: ports.sources,
            config,
            graph_locks: std::sync::Mutex::default(),
        });
        executor.register(JobKind::Analyze, Arc::clone(&runner) as Arc<dyn JobHandler>);
        executor.register(JobKind::Ingest, Arc::clone(&runner) as Arc<dyn JobHandler>);
        Ok(Self {
            runner,
            executor,
            workspace_id,
        })
    }

    /// The metadata port.
    pub(crate) fn metadata(&self) -> &Arc<dyn MetadataStore> {
        &self.runner.metadata
    }

    /// The configured bounds.
    #[must_use]
    pub fn config(&self) -> IngestionConfig {
        self.runner.config
    }

    /// Fails every job a previous process left `queued` or `running` with
    /// [`PROCESS_RESTARTED`]. Call once at startup, before serving requests.
    /// Assumes a single MindGraph core process per metadata database.
    ///
    /// # Errors
    ///
    /// Fails if the metadata store is unavailable.
    pub async fn recover_incomplete_jobs(&self) -> Result<u64, ApplicationError> {
        let error = JobError::new(
            PROCESS_RESTARTED,
            "The server restarted before the job finished. Submit the job again with a new \
             Idempotency-Key.",
        );
        let failed = self
            .runner
            .metadata
            .mark_incomplete_jobs_failed(&error)
            .await?;
        if failed > 0 {
            warn!(jobs = failed, "marked jobs cut off by a restart as failed");
        }
        Ok(failed)
    }

    async fn ensure_graph(&self, graph_id: GraphId) -> Result<(), ApplicationError> {
        self.runner
            .metadata
            .get_graph(graph_id)
            .await?
            .filter(|graph| graph.workspace_id() == self.workspace_id)
            .map(drop)
            .ok_or(ApplicationError::GraphNotFound(graph_id))
    }

    /// Starts uploading a file to a graph. The kind comes from the
    /// filename's extension (`.csv` or `.json`); the content is checked by
    /// the analysis job. The filename is metadata only.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`],
    /// [`ApplicationError::UnsupportedFormat`], or
    /// [`ApplicationError::Validation`] for an unusable filename.
    pub async fn begin_upload(
        &self,
        graph_id: GraphId,
        filename: &str,
    ) -> Result<UploadSession, ApplicationError> {
        self.ensure_graph(graph_id).await?;
        // Browsers send a bare name; other clients may send a path.
        let filename = DataSource::validate_filename(base_name(filename))?;
        let kind = DataSourceKind::from_filename(&filename).ok_or_else(|| {
            ApplicationError::UnsupportedFormat(
                "only .csv and .json files are supported".to_owned(),
            )
        })?;
        let id = DataSourceId::new();
        let writer = self.runner.sources.create(id).await?;
        Ok(UploadSession {
            runner: Arc::clone(&self.runner),
            id,
            graph_id,
            kind,
            filename,
            writer: Some(writer),
            digest: UploadDigest::new(),
        })
    }

    /// The data source of a graph in the local workspace.
    pub(crate) async fn record(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<crate::DataSourceRecord, ApplicationError> {
        self.ensure_graph(graph_id).await?;
        self.runner
            .metadata
            .get_data_source(graph_id, id)
            .await?
            .ok_or(ApplicationError::DataSourceNotFound(id))
    }

    /// Reads a data source of a graph, with its profile once analysed.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`] or
    /// [`ApplicationError::DataSourceNotFound`] (also for a data source of
    /// another graph).
    pub async fn get_data_source(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<DataSourceView, ApplicationError> {
        let record = self.record(graph_id, id).await?;
        Ok(DataSourceView {
            data_source: record.data_source,
            profile: record.profile,
        })
    }

    /// Queues an analysis job for a data source.
    ///
    /// # Errors
    ///
    /// Not-found errors, or [`ApplicationError::Unavailable`] when the job
    /// queue is full.
    pub async fn start_analysis(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<IngestionJob, ApplicationError> {
        self.record(graph_id, id).await?;
        let job = IngestionJob::analysis(graph_id, id, Utc::now().trunc_subsecs(6));
        self.enqueue(&job).await?;
        info!(job_id = %job.id, graph_id = %graph_id, data_source_id = %id, kind = "analyze", "job queued");
        Ok(job)
    }

    /// Queues an ingestion job. The mapping is validated against the active
    /// ontology, whose version is pinned on the job. Submitting the same
    /// `idempotency_key` again for the graph returns the original job.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for an invalid key,
    /// not-found errors, [`ApplicationError::Conflict`] if the data source is
    /// not analysed or the key was used for a different request,
    /// [`ApplicationError::Unprocessable`] for an invalid mapping, and
    /// [`ApplicationError::Unavailable`] when the job queue is full.
    pub async fn start_ingestion(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
        idempotency_key: &str,
        mapping: IngestionMapping,
    ) -> Result<IngestionJob, ApplicationError> {
        validate_idempotency_key(idempotency_key)?;
        let record = self.record(graph_id, id).await?;
        if let Some(existing) = self
            .runner
            .metadata
            .find_job_by_idempotency_key(graph_id, idempotency_key)
            .await?
        {
            return replay(existing, id, &mapping);
        }
        let profile = match (record.data_source.status(), record.profile) {
            (DataSourceStatus::Ready, Some(profile)) => profile,
            _ => {
                return Err(ApplicationError::Conflict(
                    "the data source must be analysed successfully before ingestion".to_owned(),
                ));
            }
        };
        let ontology = self
            .runner
            .metadata
            .get_active_ontology(graph_id)
            .await?
            .ok_or(ApplicationError::GraphNotFound(graph_id))?;
        validate_mapping(&mapping, &ontology, &profile.field_names())?;
        let job = IngestionJob::ingestion(
            graph_id,
            id,
            ontology.version(),
            idempotency_key.to_owned(),
            mapping.clone(),
            Utc::now().trunc_subsecs(6),
        );
        match self.enqueue(&job).await {
            Ok(()) => {}
            // A concurrent request with the same key won the race.
            Err(ApplicationError::Conflict(_)) => {
                let existing = self
                    .runner
                    .metadata
                    .find_job_by_idempotency_key(graph_id, idempotency_key)
                    .await?
                    .ok_or_else(|| {
                        ApplicationError::Conflict("the job could not be created".to_owned())
                    })?;
                return replay(existing, id, &mapping);
            }
            Err(other) => return Err(other),
        }
        info!(
            job_id = %job.id,
            graph_id = %graph_id,
            data_source_id = %id,
            kind = "ingest",
            ontology_version = %ontology.version(),
            "job queued"
        );
        Ok(job)
    }

    /// Persists a queued job and hands it to the executor.
    async fn enqueue(&self, job: &IngestionJob) -> Result<(), ApplicationError> {
        self.executor.submit(job).await
    }

    /// Reads a job.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::JobNotFound`], also for jobs of graphs outside
    /// the local workspace.
    pub async fn get_job(&self, id: IngestionJobId) -> Result<IngestionJob, ApplicationError> {
        let job = self
            .runner
            .metadata
            .get_job(id)
            .await?
            .ok_or(ApplicationError::JobNotFound(id))?;
        let in_workspace = self
            .runner
            .metadata
            .get_graph(job.graph_id)
            .await?
            .is_some_and(|graph| graph.workspace_id() == self.workspace_id);
        if in_workspace {
            Ok(job)
        } else {
            Err(ApplicationError::JobNotFound(id))
        }
    }
}

fn base_name(filename: &str) -> &str {
    filename.rsplit(['/', '\\']).next().unwrap_or(filename)
}

pub(crate) fn validate_idempotency_key(key: &str) -> Result<(), ApplicationError> {
    let valid = (1..=MAX_IDEMPOTENCY_KEY_CHARS).contains(&key.len())
        && key.bytes().all(|b| b.is_ascii_graphic());
    if valid {
        Ok(())
    } else {
        Err(ApplicationError::InvalidRequest(format!(
            "the Idempotency-Key must be 1 to {MAX_IDEMPOTENCY_KEY_CHARS} visible ASCII characters"
        )))
    }
}

/// The original job for a repeated idempotency key, if the request matches.
fn replay(
    existing: IngestionJob,
    data_source_id: DataSourceId,
    mapping: &IngestionMapping,
) -> Result<IngestionJob, ApplicationError> {
    if existing.data_source_id == data_source_id && existing.mapping.as_ref() == Some(mapping) {
        Ok(existing)
    } else {
        Err(ApplicationError::Conflict(
            "this Idempotency-Key was already used for a different ingestion request".to_owned(),
        ))
    }
}

/// An upload in progress. Dropping it without [`finish`](Self::finish)
/// discards the bytes.
pub struct UploadSession {
    runner: Arc<Runner>,
    id: DataSourceId,
    graph_id: GraphId,
    kind: DataSourceKind,
    filename: String,
    writer: Option<Box<dyn SourceWriter>>,
    digest: UploadDigest,
}

impl std::fmt::Debug for UploadSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadSession")
            .field("id", &self.id)
            .field("graph_id", &self.graph_id)
            .field("kind", &self.kind)
            .field("size", &self.digest.size())
            .finish_non_exhaustive()
    }
}

impl UploadSession {
    /// Appends a chunk.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::PayloadTooLarge`] once the upload exceeds the
    /// limit (the partial file is discarded), or a storage error.
    pub async fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), ApplicationError> {
        let max_bytes = self.runner.config.max_upload_bytes;
        if self.digest.size() + chunk.len() as u64 > max_bytes {
            self.writer = None;
            return Err(ApplicationError::PayloadTooLarge { max_bytes });
        }
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| ApplicationError::InvalidRequest("the upload was aborted".to_owned()))?;
        writer.write(chunk).await?;
        self.digest.update(chunk);
        Ok(())
    }

    /// Commits the file and stores the data source as `pending`.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for an empty file, or a storage
    /// or metadata error (the stored file is then removed, best effort).
    pub async fn finish(mut self) -> Result<DataSource, ApplicationError> {
        let writer = self
            .writer
            .take()
            .ok_or_else(|| ApplicationError::InvalidRequest("the upload was aborted".to_owned()))?;
        let digest = std::mem::take(&mut self.digest);
        let (size, sha256) = digest.finish();
        if size == 0 {
            return Err(ApplicationError::InvalidRequest(
                "the uploaded file is empty".to_owned(),
            ));
        }
        writer.commit().await?;
        let sources = &self.runner.sources;
        let stored = async {
            let data_source = DataSource::new(
                self.id,
                self.graph_id,
                self.kind,
                &self.filename,
                size,
                &sha256,
                Utc::now().trunc_subsecs(6),
            )?;
            self.runner
                .metadata
                .create_data_source(&data_source, &sources.storage_key(self.id))
                .await?;
            Ok::<_, ApplicationError>(data_source)
        }
        .await;
        match stored {
            Ok(data_source) => {
                info!(
                    graph_id = %self.graph_id,
                    data_source_id = %self.id,
                    kind = %self.kind,
                    size_bytes = size,
                    "data source uploaded"
                );
                Ok(data_source)
            }
            Err(err) => {
                if let Err(cleanup) = sources.delete(self.id).await {
                    warn!(data_source_id = %self.id, error = %cleanup, "could not remove an orphaned upload");
                }
                Err(err)
            }
        }
    }
}

pub(crate) fn metadata_failure(error: &MetadataStoreError) -> JobError {
    match error {
        MetadataStoreError::BackendUnavailable { .. } => JobError::new(
            "METADATA_UNAVAILABLE",
            "the metadata database is unavailable",
        ),
        _ => JobError::new("INTERNAL_ERROR", "a metadata operation failed unexpectedly"),
    }
}

fn ingestion_failure(error: &IngestionError) -> JobError {
    JobError::new(error.code(), error.public_message())
}

fn worker_crashed() -> JobError {
    JobError::new("INTERNAL_ERROR", "the dataset reader stopped unexpectedly")
}

/// Persists progress; failures only cost progress updates, not the job.
struct JobProgress {
    metadata: Arc<dyn MetadataStore>,
    id: IngestionJobId,
}

#[async_trait]
impl ProgressSink for JobProgress {
    async fn report(&self, progress: f64) {
        if let Err(err) = self.metadata.update_job_progress(self.id, progress).await {
            warn!(job_id = %self.id, error = %err, "could not persist job progress");
        }
    }
}

/// Counts the bytes read, for analysis progress.
struct CountingReader {
    inner: Box<dyn Read + Send>,
    read: Arc<AtomicU64>,
}

impl Read for CountingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buffer)?;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

#[async_trait]
impl JobHandler for Runner {
    async fn run(&self, job: &IngestionJob) -> Result<Option<JobResult>, JobError> {
        match job.kind {
            JobKind::Analyze => {
                let profile = self.analyze(job).await?;
                info!(
                    job_id = %job.id,
                    record_count = profile.record_count,
                    fields = profile.fields.len(),
                    "dataset analysed"
                );
                Ok(None)
            }
            JobKind::Ingest => {
                let summary = self.ingest(job).await?;
                info!(
                    job_id = %job.id,
                    records_processed = summary.records_processed,
                    entities_created = summary.entities_created,
                    entities_matched = summary.entities_matched,
                    relationships_created = summary.relationships_created,
                    "dataset ingested"
                );
                Ok(Some(JobResult::Ingestion(summary)))
            }
            JobKind::AiSuggest => Err(JobError::new(
                "INTERNAL_ERROR",
                "the ingestion worker cannot run AI jobs",
            )),
        }
    }

    async fn on_failure(&self, job: &IngestionJob) {
        if job.kind == JobKind::Analyze
            && let Err(err) = self
                .metadata
                .mark_data_source_failed(job.graph_id, job.data_source_id)
                .await
        {
            warn!(job_id = %job.id, error = %err, "could not mark the data source failed");
        }
    }
}

impl Runner {
    async fn source(&self, job: &IngestionJob) -> Result<crate::DataSourceRecord, JobError> {
        self.metadata
            .get_data_source(job.graph_id, job.data_source_id)
            .await
            .map_err(|err| metadata_failure(&err))?
            .ok_or_else(|| {
                JobError::new("DATA_SOURCE_NOT_FOUND", "the data source no longer exists")
            })
    }

    async fn analyze(&self, job: &IngestionJob) -> Result<DatasetProfile, JobError> {
        let record = self.source(job).await?;
        let reader = self
            .sources
            .open(job.data_source_id)
            .await
            .map_err(|err| ingestion_failure(&err))?;
        let read = Arc::new(AtomicU64::new(0));
        let reader = CountingReader {
            inner: reader,
            read: Arc::clone(&read),
        };
        let total = record.data_source.size_bytes().max(1);
        let (kind, limits, id) = (record.data_source.kind(), self.config.limits, job.id);
        let metadata = Arc::clone(&self.metadata);
        let runtime = tokio::runtime::Handle::current();
        let profile = tokio::task::spawn_blocking(move || {
            profile_dataset(kind, reader, &limits, &mut |_records| {
                #[allow(clippy::cast_precision_loss)] // A progress ratio.
                let fraction = read.load(Ordering::Relaxed) as f64 / total as f64;
                // Bounded: once per 1,000 records. Reaches 1.0 on completion.
                let _ = runtime.block_on(metadata.update_job_progress(id, fraction.min(0.99)));
            })
        })
        .await
        .map_err(|_| worker_crashed())?
        .map_err(|err| ingestion_failure(&err))?;
        self.metadata
            .mark_data_source_ready(job.graph_id, job.data_source_id, &profile)
            .await
            .map_err(|err| metadata_failure(&err))?;
        Ok(profile)
    }

    fn graph_lock(&self, graph_id: GraphId) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .graph_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(locks.entry(graph_id).or_default())
    }

    async fn ingest(&self, job: &IngestionJob) -> Result<IngestionSummary, JobError> {
        let lock = self.graph_lock(job.graph_id);
        let _exclusive = lock.lock().await;
        let record = self.source(job).await?;
        let profile = record.profile.ok_or_else(|| {
            JobError::new("DATA_SOURCE_NOT_READY", "the data source has no profile")
        })?;
        let (Some(version), Some(mapping)) = (job.ontology_version, job.mapping.as_ref()) else {
            return Err(JobError::new(
                "INTERNAL_ERROR",
                "the job has no pinned ontology or mapping",
            ));
        };
        // The pinned version, not whichever version is active now.
        let ontology = self
            .metadata
            .get_ontology(job.graph_id, version)
            .await
            .map_err(|err| metadata_failure(&err))?
            .ok_or_else(|| {
                JobError::new(
                    "ONTOLOGY_NOT_FOUND",
                    "the pinned ontology version is missing",
                )
            })?;
        let mapping = validate_mapping(mapping, &ontology, &profile.field_names())
            .map_err(|err| ingestion_failure(&err))?;
        let run = IngestionRun {
            graph_id: job.graph_id,
            data_source_id: job.data_source_id,
            kind: record.data_source.kind(),
            mapping: Arc::new(mapping),
            limits: self.config.limits,
            batch_size: self.config.batch_size,
        };
        let progress = JobProgress {
            metadata: Arc::clone(&self.metadata),
            id: job.id,
        };
        let summary = run_ingestion(&run, self.graph.as_ref(), self.sources.as_ref(), &progress)
            .await
            .map_err(|err| ingestion_failure(&err))?;
        // Activate before completing, so a completed job implies an active graph.
        self.metadata
            .activate_graph(job.graph_id)
            .await
            .map_err(|err| metadata_failure(&err))?;
        Ok(summary)
    }
}
