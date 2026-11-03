//! The `GraphStore` trait: the only way application code touches graph data.

use async_trait::async_trait;
use mindgraph_domain::{Entity, EntityId, GraphId, Relationship};

use crate::{EntitySearch, GraphPath, GraphStoreError, NeighborQuery, PathQuery, Subgraph};

/// Storage for entities and relationships, partitioned by [`GraphId`].
///
/// The trait is object safe and `Send + Sync`, so it can be shared as
/// `Arc<dyn GraphStore>`.
///
/// # Contract
///
/// Every implementation must behave as described here. A shared test suite
/// (`crates/mindgraph-graph/tests/contract`) runs against each backend.
///
/// - **Graph isolation.** Every read and write is scoped to one `GraphId`.
///   Data of one graph never appears in, and is never changed by, operations
///   on another graph. An entity that exists only in another graph is
///   treated as missing.
/// - **Identity.** An entity is identified by `GraphId + EntityId`, and a
///   relationship by `GraphId + RelationshipId`. Writes never overwrite.
///   Creating an id that already exists is a
///   [`Conflict`](GraphStoreError::Conflict).
/// - **No schema checks.** The store does not validate entities against an
///   ontology. That is the application layer's job (see
///   `Ontology::validate_entity`).
/// - **Determinism.** Lists are ordered by id (`EntityId`,
///   `RelationshipId`), so the same data always yields the same result.
/// - **Batches.** A batch is validated as a whole before anything is
///   written: mixed graphs, duplicate ids, conflicts and missing endpoints
///   reject the entire batch. If the backend fails *while* writing, the
///   error is reported but earlier chunks may already be stored. There is no
///   cross-backend all-or-nothing transaction.
/// - **Bounded work.** Query objects cap result sizes and traversal depth.
#[async_trait]
pub trait GraphStore: Send + Sync {
    /// Stores one entity.
    ///
    /// # Errors
    ///
    /// [`Conflict`](GraphStoreError::Conflict) if the entity already exists
    /// in its graph.
    async fn create_entity(&self, entity: Entity) -> Result<(), GraphStoreError>;

    /// Stores several entities of one graph. An empty batch does nothing.
    ///
    /// # Errors
    ///
    /// [`InvalidInput`](GraphStoreError::InvalidInput) if the entities belong
    /// to different graphs. [`Conflict`](GraphStoreError::Conflict) if an id
    /// repeats inside the batch or already exists.
    async fn create_entities(&self, entities: Vec<Entity>) -> Result<(), GraphStoreError>;

    /// Stores one relationship. Both endpoints must already exist in
    /// `relationship.graph_id()`. Self-loops are allowed.
    ///
    /// # Errors
    ///
    /// [`EntityNotFound`](GraphStoreError::EntityNotFound) if an endpoint is
    /// missing from the relationship's graph, which includes endpoints that
    /// live in another graph. [`Conflict`](GraphStoreError::Conflict) if the
    /// relationship already exists.
    async fn create_relationship(&self, relationship: Relationship) -> Result<(), GraphStoreError>;

    /// Stores several relationships of one graph. An empty batch does
    /// nothing.
    ///
    /// # Errors
    ///
    /// As [`create_relationship`](Self::create_relationship), plus
    /// [`InvalidInput`](GraphStoreError::InvalidInput) for mixed-graph
    /// batches and [`Conflict`](GraphStoreError::Conflict) for ids repeated
    /// inside the batch. All endpoints are checked before any duplicate.
    async fn create_relationships(
        &self,
        relationships: Vec<Relationship>,
    ) -> Result<(), GraphStoreError>;

    /// Reads one entity. A missing entity is `Ok(None)`, not an error.
    ///
    /// # Errors
    ///
    /// Backend failures only.
    async fn get_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<Option<Entity>, GraphStoreError>;

    /// Finds entities matching `search`, ordered by `EntityId`, at most
    /// `search.limit()` of them.
    ///
    /// # Errors
    ///
    /// Backend failures only.
    async fn search_entities(
        &self,
        graph_id: GraphId,
        search: &EntitySearch,
    ) -> Result<Vec<Entity>, GraphStoreError>;

    /// Returns the neighbourhood of an entity: the entity itself, the
    /// matching relationships (at most `query.limit()`, smallest ids first)
    /// and the entities at their other ends.
    ///
    /// # Errors
    ///
    /// [`EntityNotFound`](GraphStoreError::EntityNotFound) if the entity does
    /// not exist in the graph.
    async fn get_neighbors(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
        query: &NeighborQuery,
    ) -> Result<Subgraph, GraphStoreError>;

    /// Finds a shortest path (fewest hops) from `source` to `target`, or
    /// `Ok(None)` if there is none within `query.max_depth()` hops.
    ///
    /// A path from an entity to itself has zero hops. When several
    /// relationships connect two consecutive entities of the path, the one
    /// with the smallest `RelationshipId` is used. When several *different*
    /// shortest paths exist, which one is returned is backend-defined but
    /// stable for the same data.
    ///
    /// # Errors
    ///
    /// [`EntityNotFound`](GraphStoreError::EntityNotFound) if `source` or
    /// `target` does not exist in the graph.
    async fn find_path(
        &self,
        graph_id: GraphId,
        source: EntityId,
        target: EntityId,
        query: &PathQuery,
    ) -> Result<Option<GraphPath>, GraphStoreError>;

    /// Returns the requested entities that exist in the graph, plus every
    /// relationship whose two endpoints are both among them. Missing and
    /// repeated ids are ignored. No other entities are included.
    ///
    /// # Errors
    ///
    /// [`InvalidInput`](GraphStoreError::InvalidInput) if more than
    /// [`MAX_SUBGRAPH_ENTITIES`](crate::MAX_SUBGRAPH_ENTITIES) distinct ids
    /// are requested.
    async fn get_subgraph(
        &self,
        graph_id: GraphId,
        entity_ids: &[EntityId],
    ) -> Result<Subgraph, GraphStoreError>;

    /// Deletes an entity and every relationship touching it. Returns whether
    /// the entity existed in the graph.
    ///
    /// # Errors
    ///
    /// Backend failures only.
    async fn delete_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<bool, GraphStoreError>;

    /// Deletes every entity and relationship of one graph project. Other
    /// graphs are untouched. This never wipes the physical database.
    ///
    /// # Errors
    ///
    /// Backend failures only.
    async fn clear_graph(&self, graph_id: GraphId) -> Result<(), GraphStoreError>;
}
