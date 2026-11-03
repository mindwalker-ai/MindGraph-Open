//! Graph project use cases.

use std::sync::Arc;

use chrono::{SubsecRound, Utc};
use mindgraph_domain::{
    GraphId, GraphProject, GraphStatus, Ontology, OntologyVersion, WorkspaceId,
};
use tracing::info;

use crate::{ApplicationError, MetadataStore, MetadataStoreError};

/// The number of graphs listed when no limit is given.
pub const DEFAULT_LIST_LIMIT: usize = 50;
/// The largest accepted list limit.
pub const MAX_LIST_LIMIT: usize = 100;

/// Input of [`GraphProjectService::create_graph`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGraphProject {
    /// The graph name. Trimmed, 1–120 characters.
    pub name: String,
    /// An optional description. Blank text is stored as no description.
    pub description: Option<String>,
}

/// Creates and reads graph projects in the local workspace.
///
/// Holds only a metadata port: no HTTP or database types.
#[derive(Clone)]
pub struct GraphProjectService {
    metadata: Arc<dyn MetadataStore>,
    workspace_id: WorkspaceId,
}

impl std::fmt::Debug for GraphProjectService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphProjectService")
            .field("workspace_id", &self.workspace_id)
            .finish_non_exhaustive()
    }
}

impl GraphProjectService {
    /// Ensures the local workspace exists and returns a service bound to it.
    ///
    /// # Errors
    ///
    /// Fails if the metadata store is unavailable.
    pub async fn bootstrap(metadata: Arc<dyn MetadataStore>) -> Result<Self, ApplicationError> {
        let workspace = metadata.ensure_local_workspace().await?;
        info!(workspace_id = %workspace.id(), "local workspace ready");
        Ok(Self {
            metadata,
            workspace_id: workspace.id(),
        })
    }

    /// The local workspace all graphs belong to.
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    /// Creates a draft graph project together with its empty ontology v1.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::Validation`] for an invalid name or description.
    pub async fn create_graph(
        &self,
        input: CreateGraphProject,
    ) -> Result<GraphProject, ApplicationError> {
        // Microseconds: the precision of the metadata store.
        let now = Utc::now().trunc_subsecs(6);
        let graph = GraphProject::from_parts(
            GraphId::new(),
            self.workspace_id,
            &input.name,
            input.description.as_deref(),
            GraphStatus::Draft,
            now,
            now,
        )?;
        let ontology = Ontology::new(graph.id(), OntologyVersion::INITIAL, Vec::new(), Vec::new())?;
        self.metadata.create_graph(&graph, &ontology).await?;
        info!(
            operation = "create_graph",
            graph_id = %graph.id(),
            workspace_id = %self.workspace_id,
            "graph project created"
        );
        Ok(graph)
    }

    /// Reads a graph project of the local workspace.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`] if it does not exist.
    pub async fn get_graph(&self, graph_id: GraphId) -> Result<GraphProject, ApplicationError> {
        self.metadata
            .get_graph(graph_id)
            .await?
            .filter(|graph| graph.workspace_id() == self.workspace_id)
            .ok_or(ApplicationError::GraphNotFound(graph_id))
    }

    /// Lists graph projects, newest first. `limit` defaults to
    /// [`DEFAULT_LIST_LIMIT`] and may not exceed [`MAX_LIST_LIMIT`].
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for a limit outside
    /// `1..=MAX_LIST_LIMIT`.
    pub async fn list_graphs(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<GraphProject>, ApplicationError> {
        let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT);
        if !(1..=MAX_LIST_LIMIT).contains(&limit) {
            return Err(ApplicationError::InvalidRequest(format!(
                "limit must be between 1 and {MAX_LIST_LIMIT}, got {limit}"
            )));
        }
        Ok(self.metadata.list_graphs(self.workspace_id, limit).await?)
    }

    /// Reads the active ontology of a graph project.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`] if the graph does not exist.
    pub async fn get_active_ontology(
        &self,
        graph_id: GraphId,
    ) -> Result<Ontology, ApplicationError> {
        self.get_graph(graph_id).await?;
        self.metadata
            .get_active_ontology(graph_id)
            .await?
            .ok_or_else(|| {
                ApplicationError::internal(MetadataStoreError::CorruptData(format!(
                    "graph {graph_id} has no active ontology"
                )))
            })
    }

    /// Checks that the metadata store is reachable.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::Unavailable`] when it is not.
    pub async fn check_ready(&self) -> Result<(), ApplicationError> {
        Ok(self.metadata.ping().await?)
    }
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::{GraphStatus, OntologyVersion};

    use super::*;
    use crate::MemoryMetadataStore;

    async fn service() -> GraphProjectService {
        GraphProjectService::bootstrap(Arc::new(MemoryMetadataStore::new()))
            .await
            .unwrap()
    }

    fn input(name: &str, description: Option<&str>) -> CreateGraphProject {
        CreateGraphProject {
            name: name.to_owned(),
            description: description.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn bootstrap_reuses_the_local_workspace() {
        let store: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
        let first = GraphProjectService::bootstrap(store.clone()).await.unwrap();
        let second = GraphProjectService::bootstrap(store).await.unwrap();
        assert_eq!(first.workspace_id(), second.workspace_id());
    }

    #[tokio::test]
    async fn create_graph_returns_a_draft_in_the_local_workspace() {
        let service = service().await;
        let graph = service
            .create_graph(input("  Fraud Investigation ", Some("Card fraud")))
            .await
            .unwrap();
        assert_eq!(graph.name(), "Fraud Investigation");
        assert_eq!(graph.description(), Some("Card fraud"));
        assert_eq!(graph.status(), GraphStatus::Draft);
        assert_eq!(graph.workspace_id(), service.workspace_id());
        assert_eq!(
            graph.created_at().timestamp_subsec_nanos() % 1_000,
            0,
            "timestamps use microsecond precision"
        );
    }

    #[tokio::test]
    async fn invalid_graph_names_are_validation_errors() {
        let service = service().await;
        for name in ["", "   ", &"x".repeat(121)] {
            let err = service.create_graph(input(name, None)).await.unwrap_err();
            assert!(
                matches!(err, ApplicationError::Validation(_)),
                "{name:?}: {err}"
            );
        }
        assert!(service.list_graphs(None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn new_graphs_get_an_empty_active_ontology_v1() {
        let service = service().await;
        let graph = service.create_graph(input("Cyber", None)).await.unwrap();
        let ontology = service.get_active_ontology(graph.id()).await.unwrap();
        assert_eq!(ontology.graph_id(), graph.id());
        assert_eq!(ontology.version(), OntologyVersion::INITIAL);
        assert!(ontology.entity_types().is_empty());
        assert!(ontology.relationship_types().is_empty());
    }

    #[tokio::test]
    async fn get_graph_returns_the_created_graph() {
        let service = service().await;
        let graph = service
            .create_graph(input("Knowledge", None))
            .await
            .unwrap();
        assert_eq!(service.get_graph(graph.id()).await.unwrap(), graph);
    }

    #[tokio::test]
    async fn missing_graphs_are_not_found() {
        let service = service().await;
        let id = GraphId::new();
        assert!(matches!(
            service.get_graph(id).await,
            Err(ApplicationError::GraphNotFound(missing)) if missing == id
        ));
        assert!(matches!(
            service.get_active_ontology(id).await,
            Err(ApplicationError::GraphNotFound(_))
        ));
    }

    #[tokio::test]
    async fn graphs_of_other_workspaces_are_not_visible() {
        let store: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
        let service = GraphProjectService::bootstrap(store.clone()).await.unwrap();
        // Write a graph straight into the store under a foreign workspace id.
        let now = Utc::now().trunc_subsecs(6);
        let foreign = GraphProject::from_parts(
            GraphId::new(),
            WorkspaceId::new(),
            "Foreign",
            None,
            GraphStatus::Draft,
            now,
            now,
        )
        .unwrap();
        let ontology = Ontology::new(
            foreign.id(),
            OntologyVersion::INITIAL,
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let err = store.create_graph(&foreign, &ontology).await.unwrap_err();
        assert!(matches!(err, MetadataStoreError::NotFound(_)), "{err}");
        assert!(matches!(
            service.get_graph(foreign.id()).await,
            Err(ApplicationError::GraphNotFound(_))
        ));
    }

    #[tokio::test]
    async fn list_graphs_is_newest_first_and_bounded() {
        let service = service().await;
        let mut created = Vec::new();
        for name in ["first", "second", "third"] {
            created.push(service.create_graph(input(name, None)).await.unwrap());
        }
        let listed = service.list_graphs(None).await.unwrap();
        let names: Vec<&str> = listed.iter().map(GraphProject::name).collect();
        assert_eq!(names, ["third", "second", "first"]);
        assert_eq!(service.list_graphs(Some(2)).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn list_limit_is_validated() {
        let service = service().await;
        for limit in [0, MAX_LIST_LIMIT + 1] {
            assert!(matches!(
                service.list_graphs(Some(limit)).await,
                Err(ApplicationError::InvalidRequest(_))
            ));
        }
        assert!(service.list_graphs(Some(MAX_LIST_LIMIT)).await.is_ok());
    }
}
