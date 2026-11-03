//! The in-process background job executor.
//!
//! A dispatcher takes job ids from a bounded queue and runs at most
//! `concurrency` jobs at a time (a semaphore). Each [`JobKind`] it runs has
//! one [`JobHandler`]; the executor owns the lifecycle
//! (`queued → running → completed | failed`) and persists every transition
//! through the [`MetadataStore`]. Analysis and ingestion share one executor;
//! AI jobs have their own, so slow model calls never hold up dataset jobs.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Instant;

use async_trait::async_trait;
use mindgraph_domain::IngestionJobId;
use tokio::sync::{Semaphore, mpsc};
use tracing::{info, warn};

use crate::{ApplicationError, IngestionJob, JobError, JobKind, JobResult, MetadataStore};

/// Does the work of one job kind.
#[async_trait]
pub(crate) trait JobHandler: Send + Sync {
    /// Runs a job that is already `running`. `Ok` completes it with the
    /// result, `Err` fails it with a client-safe error.
    async fn run(&self, job: &IngestionJob) -> Result<Option<JobResult>, JobError>;

    /// Called after a failure, before it is recorded (for example to mark a
    /// data source failed).
    async fn on_failure(&self, _job: &IngestionJob) {}
}

type Handlers = RwLock<HashMap<JobKind, Arc<dyn JobHandler>>>;

/// What the dispatcher needs. Holds no queue sender, so dropping every
/// [`JobExecutor`] stops the dispatcher.
struct Shared {
    metadata: Arc<dyn MetadataStore>,
    handlers: Handlers,
}

/// Submits jobs and runs them in the background. Cheap to clone.
#[derive(Clone)]
pub(crate) struct JobExecutor {
    shared: Arc<Shared>,
    queue: mpsc::Sender<IngestionJobId>,
}

impl JobExecutor {
    /// Starts the dispatcher. Must be called inside a Tokio runtime.
    pub(crate) fn start(
        metadata: Arc<dyn MetadataStore>,
        concurrency: usize,
        queue_capacity: usize,
    ) -> Self {
        let shared = Arc::new(Shared {
            metadata,
            handlers: RwLock::default(),
        });
        let (queue, receiver) = mpsc::channel(queue_capacity.max(1));
        tokio::spawn(dispatch(Arc::clone(&shared), receiver, concurrency.max(1)));
        Self { shared, queue }
    }

    /// Registers the handler of a job kind, replacing any earlier one.
    pub(crate) fn register(&self, kind: JobKind, handler: Arc<dyn JobHandler>) {
        self.shared
            .handlers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(kind, handler);
    }

    /// Persists a queued job and hands it to the dispatcher. The queue slot
    /// is reserved first, so a full queue stores nothing.
    pub(crate) async fn submit(&self, job: &IngestionJob) -> Result<(), ApplicationError> {
        let permit = self
            .queue
            .try_reserve()
            .map_err(|_| ApplicationError::unavailable("the job queue is full; try again later"))?;
        self.shared.metadata.create_job(job).await?;
        permit.send(job.id);
        Ok(())
    }
}

async fn dispatch(
    shared: Arc<Shared>,
    mut queue: mpsc::Receiver<IngestionJobId>,
    concurrency: usize,
) {
    let workers = Arc::new(Semaphore::new(concurrency));
    while let Some(id) = queue.recv().await {
        let Ok(permit) = Arc::clone(&workers).acquire_owned().await else {
            break;
        };
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            // A panicking job must not stay `running`: run it in its own
            // task and fail it if that task dies.
            let job = tokio::spawn({
                let shared = Arc::clone(&shared);
                async move { run_job(&shared, id).await }
            });
            if job.await.is_err() {
                let error = JobError::new("INTERNAL_ERROR", "the job stopped unexpectedly");
                if let Err(err) = shared.metadata.fail_job(id, &error).await {
                    warn!(job_id = %id, error = %err, "could not record a crashed job");
                }
            }
            drop(permit);
        });
    }
}

async fn run_job(shared: &Shared, id: IngestionJobId) {
    let job = match shared.metadata.start_job(id).await {
        Ok(job) => job,
        Err(err) => {
            // For example failed by restart recovery while queued.
            warn!(job_id = %id, error = %err, "job could not start");
            return;
        }
    };
    let handler = shared
        .handlers
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&job.kind)
        .cloned();
    let Some(handler) = handler else {
        let error = JobError::new("INTERNAL_ERROR", "no worker handles this job kind");
        if let Err(err) = shared.metadata.fail_job(id, &error).await {
            warn!(job_id = %id, error = %err, "could not record a failed job");
        }
        return;
    };
    let started = Instant::now();
    info!(
        job_id = %id,
        graph_id = %job.graph_id,
        data_source_id = %job.data_source_id,
        kind = %job.kind,
        "job started"
    );
    let outcome = handler.run(&job).await;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match outcome {
        Ok(result) => match shared.metadata.complete_job(id, result.as_ref()).await {
            Ok(_) => info!(job_id = %id, kind = %job.kind, duration_ms, "job completed"),
            Err(err) => {
                // For example a result the store refuses. A job left
                // `running` would never reach a terminal state for clients.
                warn!(job_id = %id, error = %err, "could not record a completed job");
                let error = JobError::new(
                    "RESULT_NOT_RECORDED",
                    "the job finished but its result could not be stored",
                );
                if let Err(err) = shared.metadata.fail_job(id, &error).await {
                    warn!(job_id = %id, error = %err, "could not record a failed job");
                }
            }
        },
        Err(error) => {
            handler.on_failure(&job).await;
            warn!(job_id = %id, kind = %job.kind, code = %error.code, duration_ms, "job failed");
            if let Err(err) = shared.metadata.fail_job(id, &error).await {
                warn!(job_id = %id, error = %err, "could not record a failed job");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{SubsecRound, Utc};
    use mindgraph_domain::{DataSource, DataSourceId, DataSourceKind};
    use mindgraph_ingestion::IngestionSummary;

    use super::*;
    use crate::{CreateGraphProject, GraphProjectService, JobStatus, MemoryMetadataStore};

    /// Returns an ingestion result, which the store refuses for an analysis
    /// job: a result that cannot be recorded.
    struct WrongResult;

    #[async_trait]
    impl JobHandler for WrongResult {
        async fn run(&self, _: &IngestionJob) -> Result<Option<JobResult>, JobError> {
            Ok(Some(JobResult::Ingestion(IngestionSummary::default())))
        }
    }

    #[tokio::test]
    async fn a_result_that_cannot_be_recorded_fails_the_job() {
        let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
        let graphs = GraphProjectService::bootstrap(Arc::clone(&metadata))
            .await
            .unwrap();
        let graph = graphs
            .create_graph(CreateGraphProject {
                name: "Jobs".into(),
                description: None,
            })
            .await
            .unwrap();
        let now = Utc::now().trunc_subsecs(6);
        let source = DataSource::new(
            DataSourceId::new(),
            graph.id(),
            DataSourceKind::Csv,
            "a.csv",
            1,
            &"0".repeat(64),
            now,
        )
        .unwrap();
        metadata.create_data_source(&source, "a").await.unwrap();
        let executor = JobExecutor::start(Arc::clone(&metadata), 1, 1);
        executor.register(JobKind::Analyze, Arc::new(WrongResult));
        let job = IngestionJob::analysis(graph.id(), source.id(), now);
        executor.submit(&job).await.unwrap();

        let mut stored = metadata.get_job(job.id).await.unwrap().unwrap();
        for _ in 0..400 {
            if !matches!(stored.status, JobStatus::Queued | JobStatus::Running) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
            stored = metadata.get_job(job.id).await.unwrap().unwrap();
        }
        assert_eq!(stored.status, JobStatus::Failed);
        assert_eq!(stored.error.unwrap().code, "RESULT_NOT_RECORDED");
    }
}
