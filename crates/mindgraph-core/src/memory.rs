//! The in-memory [`MetadataStore`]: the reference behaviour of the contract.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, SubsecRound, Utc};
use mindgraph_domain::{
    DataSource, DataSourceId, DataSourceStatus, DomainError, GraphId, GraphProject, GraphStatus,
    IngestionJobId, Ontology, OntologyVersion, Workspace, WorkspaceId,
};
use mindgraph_ingestion::{DatasetProfile, IngestionSummary};

use crate::metadata::{DataSourceRecord, LOCAL_WORKSPACE_NAME, MetadataStore, MetadataStoreError};
use crate::{IngestionJob, JobError, JobStatus};

/// An in-memory [`MetadataStore`]: the reference behaviour of the contract,
/// for tests and local development. Data is lost when the process exits.
#[derive(Debug, Default)]
pub struct MemoryMetadataStore {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    local_workspace: Option<WorkspaceId>,
    workspaces: BTreeMap<WorkspaceId, Workspace>,
    /// Each graph with its active ontology version.
    graphs: BTreeMap<GraphId, (GraphProject, OntologyVersion)>,
    ontologies: BTreeMap<(GraphId, OntologyVersion), Ontology>,
    data_sources: BTreeMap<DataSourceId, DataSourceRecord>,
    jobs: BTreeMap<IngestionJobId, IngestionJob>,
}

impl MemoryMetadataStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // Every method validates before mutating, so state behind a poisoned
    // lock is still consistent.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn domain_failure(error: &DomainError) -> MetadataStoreError {
    MetadataStoreError::BackendError {
        message: error.to_string(),
        source: None,
    }
}

/// `now` at storage precision, never earlier than `previous`.
fn touch(previous: DateTime<Utc>) -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6).max(previous)
}

fn with_graph_status(
    graph: &GraphProject,
    status: GraphStatus,
) -> Result<GraphProject, MetadataStoreError> {
    GraphProject::from_parts(
        graph.id(),
        graph.workspace_id(),
        graph.name(),
        graph.description(),
        status,
        graph.created_at(),
        touch(graph.updated_at()),
    )
    .map_err(|err| domain_failure(&err))
}

fn with_source_status(
    source: &DataSource,
    status: DataSourceStatus,
) -> Result<DataSource, MetadataStoreError> {
    DataSource::from_parts(
        source.id(),
        source.graph_id(),
        source.kind(),
        source.filename(),
        status,
        source.size_bytes(),
        source.content_sha256(),
        source.created_at(),
        touch(source.updated_at()),
    )
    .map_err(|err| domain_failure(&err))
}

fn job_mut<'a>(
    state: &'a mut State,
    id: IngestionJobId,
    allowed: &[JobStatus],
    action: &str,
) -> Result<&'a mut IngestionJob, MetadataStoreError> {
    let job = state
        .jobs
        .get_mut(&id)
        .ok_or_else(|| MetadataStoreError::NotFound(format!("job {id}")))?;
    if !allowed.contains(&job.status) {
        return Err(MetadataStoreError::Conflict(format!(
            "cannot {action} job {id}: it is {}",
            job.status
        )));
    }
    Ok(job)
}

#[async_trait]
impl MetadataStore for MemoryMetadataStore {
    async fn ensure_local_workspace(&self) -> Result<Workspace, MetadataStoreError> {
        let mut state = self.state();
        if let Some(workspace) = state
            .local_workspace
            .and_then(|id| state.workspaces.get(&id))
        {
            return Ok(workspace.clone());
        }
        let now = Utc::now().trunc_subsecs(6);
        let workspace = Workspace::from_parts(WorkspaceId::new(), LOCAL_WORKSPACE_NAME, now, now)
            .map_err(|err| domain_failure(&err))?;
        state.local_workspace = Some(workspace.id());
        state.workspaces.insert(workspace.id(), workspace.clone());
        Ok(workspace)
    }

    async fn get_workspace(
        &self,
        id: WorkspaceId,
    ) -> Result<Option<Workspace>, MetadataStoreError> {
        Ok(self.state().workspaces.get(&id).cloned())
    }

    async fn create_graph(
        &self,
        graph: &GraphProject,
        initial_ontology: &Ontology,
    ) -> Result<(), MetadataStoreError> {
        if initial_ontology.graph_id() != graph.id() {
            return Err(MetadataStoreError::InvalidInput(format!(
                "initial ontology belongs to graph {}, not {}",
                initial_ontology.graph_id(),
                graph.id()
            )));
        }
        let mut state = self.state();
        if !state.workspaces.contains_key(&graph.workspace_id()) {
            return Err(MetadataStoreError::NotFound(format!(
                "workspace {}",
                graph.workspace_id()
            )));
        }
        if state.graphs.contains_key(&graph.id()) {
            return Err(MetadataStoreError::Conflict(format!(
                "graph {} already exists",
                graph.id()
            )));
        }
        let version = initial_ontology.version();
        state.graphs.insert(graph.id(), (graph.clone(), version));
        state
            .ontologies
            .insert((graph.id(), version), initial_ontology.clone());
        Ok(())
    }

    async fn get_graph(&self, id: GraphId) -> Result<Option<GraphProject>, MetadataStoreError> {
        Ok(self.state().graphs.get(&id).map(|(graph, _)| graph.clone()))
    }

    async fn list_graphs(
        &self,
        workspace_id: WorkspaceId,
        limit: usize,
    ) -> Result<Vec<GraphProject>, MetadataStoreError> {
        let state = self.state();
        let mut graphs: Vec<GraphProject> = state
            .graphs
            .values()
            .map(|(graph, _)| graph)
            .filter(|graph| graph.workspace_id() == workspace_id)
            .cloned()
            .collect();
        graphs.sort_by(|a, b| {
            b.created_at()
                .cmp(&a.created_at())
                .then_with(|| b.id().cmp(&a.id()))
        });
        graphs.truncate(limit);
        Ok(graphs)
    }

    async fn activate_graph(&self, id: GraphId) -> Result<GraphProject, MetadataStoreError> {
        let mut state = self.state();
        let (graph, _) = state
            .graphs
            .get_mut(&id)
            .ok_or_else(|| MetadataStoreError::NotFound(format!("graph {id}")))?;
        if graph.status() == GraphStatus::Draft {
            *graph = with_graph_status(graph, GraphStatus::Active)?;
        }
        Ok(graph.clone())
    }

    async fn create_ontology(&self, ontology: &Ontology) -> Result<(), MetadataStoreError> {
        let mut state = self.state();
        let key = (ontology.graph_id(), ontology.version());
        if !state.graphs.contains_key(&key.0) {
            return Err(MetadataStoreError::NotFound(format!("graph {}", key.0)));
        }
        if state.ontologies.contains_key(&key) {
            return Err(MetadataStoreError::Conflict(format!(
                "ontology {} of graph {} already exists",
                key.1, key.0
            )));
        }
        state.ontologies.insert(key, ontology.clone());
        Ok(())
    }

    async fn create_and_activate_ontology(
        &self,
        ontology: &Ontology,
    ) -> Result<(), MetadataStoreError> {
        // One lock for both steps: atomic, like the PostgreSQL transaction.
        let mut state = self.state();
        let (graph_id, version) = (ontology.graph_id(), ontology.version());
        let Some((graph, _)) = state.graphs.get(&graph_id) else {
            return Err(MetadataStoreError::NotFound(format!("graph {graph_id}")));
        };
        if state.ontologies.contains_key(&(graph_id, version)) {
            return Err(MetadataStoreError::Conflict(format!(
                "ontology {version} of graph {graph_id} already exists"
            )));
        }
        let updated = with_graph_status(graph, graph.status())?;
        state
            .ontologies
            .insert((graph_id, version), ontology.clone());
        state.graphs.insert(graph_id, (updated, version));
        Ok(())
    }

    async fn get_ontology(
        &self,
        graph_id: GraphId,
        version: OntologyVersion,
    ) -> Result<Option<Ontology>, MetadataStoreError> {
        Ok(self.state().ontologies.get(&(graph_id, version)).cloned())
    }

    async fn get_active_ontology(
        &self,
        graph_id: GraphId,
    ) -> Result<Option<Ontology>, MetadataStoreError> {
        let state = self.state();
        let Some((_, version)) = state.graphs.get(&graph_id) else {
            return Ok(None);
        };
        match state.ontologies.get(&(graph_id, *version)) {
            Some(ontology) => Ok(Some(ontology.clone())),
            None => Err(MetadataStoreError::CorruptData(format!(
                "graph {graph_id} points to missing ontology {version}"
            ))),
        }
    }

    async fn set_active_ontology(
        &self,
        graph_id: GraphId,
        version: OntologyVersion,
    ) -> Result<(), MetadataStoreError> {
        let mut state = self.state();
        if !state.ontologies.contains_key(&(graph_id, version)) {
            return Err(MetadataStoreError::NotFound(format!(
                "ontology {version} of graph {graph_id}"
            )));
        }
        let Some((graph, active)) = state.graphs.get_mut(&graph_id) else {
            return Err(MetadataStoreError::NotFound(format!("graph {graph_id}")));
        };
        *graph = with_graph_status(graph, graph.status())?;
        *active = version;
        Ok(())
    }

    async fn create_data_source(
        &self,
        data_source: &DataSource,
        storage_key: &str,
    ) -> Result<(), MetadataStoreError> {
        let mut state = self.state();
        if !state.graphs.contains_key(&data_source.graph_id()) {
            return Err(MetadataStoreError::NotFound(format!(
                "graph {}",
                data_source.graph_id()
            )));
        }
        if state.data_sources.contains_key(&data_source.id()) {
            return Err(MetadataStoreError::Conflict(format!(
                "data source {} already exists",
                data_source.id()
            )));
        }
        state.data_sources.insert(
            data_source.id(),
            DataSourceRecord {
                data_source: data_source.clone(),
                storage_key: storage_key.to_owned(),
                profile: None,
            },
        );
        Ok(())
    }

    async fn get_data_source(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<Option<DataSourceRecord>, MetadataStoreError> {
        Ok(self
            .state()
            .data_sources
            .get(&id)
            .filter(|record| record.data_source.graph_id() == graph_id)
            .cloned())
    }

    async fn mark_data_source_ready(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
        profile: &DatasetProfile,
    ) -> Result<DataSource, MetadataStoreError> {
        let mut state = self.state();
        let record = data_source_mut(&mut state, graph_id, id)?;
        record.data_source = with_source_status(&record.data_source, DataSourceStatus::Ready)?;
        record.profile = Some(profile.clone());
        Ok(record.data_source.clone())
    }

    async fn mark_data_source_failed(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<DataSource, MetadataStoreError> {
        let mut state = self.state();
        let record = data_source_mut(&mut state, graph_id, id)?;
        record.data_source = with_source_status(&record.data_source, DataSourceStatus::Failed)?;
        record.profile = None;
        Ok(record.data_source.clone())
    }

    async fn create_job(&self, job: &IngestionJob) -> Result<(), MetadataStoreError> {
        job.validate_new()
            .map_err(MetadataStoreError::InvalidInput)?;
        let mut state = self.state();
        let source_in_graph = state
            .data_sources
            .get(&job.data_source_id)
            .is_some_and(|record| record.data_source.graph_id() == job.graph_id);
        if !source_in_graph {
            return Err(MetadataStoreError::NotFound(format!(
                "data source {} of graph {}",
                job.data_source_id, job.graph_id
            )));
        }
        if let Some(version) = job.ontology_version
            && !state.ontologies.contains_key(&(job.graph_id, version))
        {
            return Err(MetadataStoreError::InvalidInput(format!(
                "ontology {version} of graph {} does not exist",
                job.graph_id
            )));
        }
        if state.jobs.contains_key(&job.id) {
            return Err(MetadataStoreError::Conflict(format!(
                "job {} already exists",
                job.id
            )));
        }
        if let Some(key) = &job.idempotency_key
            && state.jobs.values().any(|other| {
                other.graph_id == job.graph_id && other.idempotency_key.as_ref() == Some(key)
            })
        {
            return Err(MetadataStoreError::Conflict(format!(
                "graph {} already has a job with this idempotency key",
                job.graph_id
            )));
        }
        state.jobs.insert(job.id, job.clone());
        Ok(())
    }

    async fn get_job(
        &self,
        id: IngestionJobId,
    ) -> Result<Option<IngestionJob>, MetadataStoreError> {
        Ok(self.state().jobs.get(&id).cloned())
    }

    async fn find_job_by_idempotency_key(
        &self,
        graph_id: GraphId,
        key: &str,
    ) -> Result<Option<IngestionJob>, MetadataStoreError> {
        Ok(self
            .state()
            .jobs
            .values()
            .find(|job| job.graph_id == graph_id && job.idempotency_key.as_deref() == Some(key))
            .cloned())
    }

    async fn start_job(&self, id: IngestionJobId) -> Result<IngestionJob, MetadataStoreError> {
        let mut state = self.state();
        let job = job_mut(&mut state, id, &[JobStatus::Queued], "start")?;
        job.status = JobStatus::Running;
        job.updated_at = touch(job.updated_at);
        Ok(job.clone())
    }

    async fn update_job_progress(
        &self,
        id: IngestionJobId,
        progress: f64,
    ) -> Result<(), MetadataStoreError> {
        if progress.is_nan() {
            return Err(MetadataStoreError::InvalidInput(
                "progress must be a number".to_owned(),
            ));
        }
        let mut state = self.state();
        let job = job_mut(&mut state, id, &[JobStatus::Running], "update progress of")?;
        let progress = progress.clamp(0.0, 1.0);
        if progress > job.progress {
            job.progress = progress;
            job.updated_at = touch(job.updated_at);
        }
        Ok(())
    }

    async fn complete_job(
        &self,
        id: IngestionJobId,
        result: Option<&IngestionSummary>,
    ) -> Result<IngestionJob, MetadataStoreError> {
        let mut state = self.state();
        let job = job_mut(&mut state, id, &[JobStatus::Running], "complete")?;
        job.status = JobStatus::Completed;
        job.progress = 1.0;
        job.result = result.copied();
        job.updated_at = touch(job.updated_at);
        Ok(job.clone())
    }

    async fn fail_job(
        &self,
        id: IngestionJobId,
        error: &JobError,
    ) -> Result<IngestionJob, MetadataStoreError> {
        let mut state = self.state();
        let job = job_mut(
            &mut state,
            id,
            &[JobStatus::Queued, JobStatus::Running],
            "fail",
        )?;
        job.status = JobStatus::Failed;
        job.error = Some(error.clone());
        job.updated_at = touch(job.updated_at);
        Ok(job.clone())
    }

    async fn mark_incomplete_jobs_failed(
        &self,
        error: &JobError,
    ) -> Result<u64, MetadataStoreError> {
        let mut state = self.state();
        let mut changed = 0;
        for job in state.jobs.values_mut() {
            if matches!(job.status, JobStatus::Queued | JobStatus::Running) {
                job.status = JobStatus::Failed;
                job.error = Some(error.clone());
                job.updated_at = touch(job.updated_at);
                changed += 1;
            }
        }
        Ok(changed)
    }

    async fn ping(&self) -> Result<(), MetadataStoreError> {
        Ok(())
    }
}

fn data_source_mut(
    state: &mut State,
    graph_id: GraphId,
    id: DataSourceId,
) -> Result<&mut DataSourceRecord, MetadataStoreError> {
    state
        .data_sources
        .get_mut(&id)
        .filter(|record| record.data_source.graph_id() == graph_id)
        .ok_or_else(|| {
            MetadataStoreError::NotFound(format!("data source {id} of graph {graph_id}"))
        })
}
