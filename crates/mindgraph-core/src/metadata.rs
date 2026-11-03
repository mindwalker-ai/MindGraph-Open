//! The metadata port: persistence of control-plane data (ADR-005).
//!
//! Workspaces, graph projects and ontology versions live here. Graph
//! topology does not: entities and relationships belong to the `GraphStore`.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use mindgraph_domain::{GraphId, GraphProject, Ontology, OntologyVersion, Workspace, WorkspaceId};

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

    /// The write would duplicate an existing record.
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

/// Persistence of workspaces, graph projects and ontology versions.
///
/// Object safe and `Send + Sync`, so it can be shared as
/// `Arc<dyn MetadataStore>`.
///
/// # Contract
///
/// - Records are never overwritten by create operations. A duplicate is a
///   [`Conflict`](MetadataStoreError::Conflict).
/// - Ontology versions are immutable. New versions are added with
///   [`create_ontology`](Self::create_ontology).
/// - A graph project always has an active ontology version that exists.
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

    /// Stores a new ontology version for an existing graph. Does not change
    /// which version is active.
    ///
    /// # Errors
    ///
    /// [`NotFound`](MetadataStoreError::NotFound) if the graph does not exist,
    /// [`Conflict`](MetadataStoreError::Conflict) if the version exists.
    async fn create_ontology(&self, ontology: &Ontology) -> Result<(), MetadataStoreError>;

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

    /// Checks that the backend is reachable (used for readiness).
    async fn ping(&self) -> Result<(), MetadataStoreError>;
}

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

fn domain_failure(error: &mindgraph_domain::DomainError) -> MetadataStoreError {
    MetadataStoreError::BackendError {
        message: error.to_string(),
        source: None,
    }
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
        let now = Utc::now().trunc_subsecs(6).max(graph.updated_at());
        *graph = GraphProject::from_parts(
            graph.id(),
            graph.workspace_id(),
            graph.name(),
            graph.description(),
            graph.status(),
            graph.created_at(),
            now,
        )
        .map_err(|err| domain_failure(&err))?;
        *active = version;
        Ok(())
    }

    async fn ping(&self) -> Result<(), MetadataStoreError> {
        Ok(())
    }
}
