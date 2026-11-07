//! Apache HugeGraph 1.7 adapter.
//!
//! Targets the REST API of HugeGraph **1.7.0** (`/graphspaces/{graphspace}/
//! graphs/{graph}/...`), as shipped in the `hugegraph/hugegraph:1.7.0`
//! image. The physical mapping is described in [`codec`].

mod client;
mod codec;
mod config;
mod dto;
mod schema;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use async_trait::async_trait;
use mindgraph_domain::{Entity, EntityId, GraphId, Relationship, RelationshipId};
use reqwest::StatusCode;
use serde_json::{Map, Value};
use tracing::debug;

pub use config::HugeGraphConfig;

use self::client::{Client, Failure};
use self::codec::{EDGE_LABEL, PROP_GRAPH_ID, PROP_ID, PROP_TYPE, VERTEX_LABEL};
use self::dto::{EdgePage, Vertex, VertexPage};
use crate::validate;
use crate::{
    Direction, EntitySearch, GraphPath, GraphStore, GraphStoreError, NeighborQuery, PathQuery,
    Subgraph,
};

/// Items per batch write (HugeGraph's default `batch.max_*_per_batch`).
const WRITE_BATCH: usize = 500;
/// Ids per lookup request; keeps request URLs far below HTTP header limits.
const ID_LOOKUP_CHUNK: usize = 50;
/// Page size for paged scans.
const PAGE_SIZE: usize = 500;
/// Upper bound on candidates scanned by one search or adjacency read.
const MAX_SCAN: usize = 100_000;
/// Most entities whose relationships one path search reads (one request
/// each); beyond it the search stops with `InvalidInput` instead of running
/// for minutes.
const MAX_PATH_EXPANSIONS: usize = 2_000;

/// A [`GraphStore`] backed by Apache HugeGraph 1.7.
///
/// Uses one generic physical schema for every graph project: entities are
/// vertices labelled `mindgraph_entity` with id `"{graph_id}:{entity_id}"`,
/// and relationships are edges labelled `mindgraph_relationship`. Call
/// [`ensure_schema`](Self::ensure_schema) once at startup.
///
/// Notes on the HugeGraph implementation:
///
/// - HugeGraph silently overwrites an element written with an existing id,
///   so duplicates are detected with a lookup before each write. This
///   check-then-write is not atomic across concurrent writers.
/// - Exact property filters are applied in Rust after an indexed scan by
///   graph and entity type, because user properties are stored in one
///   encoded field (`mg_properties`). Scans are capped at 100,000
///   candidates.
/// - `clear_graph` deletes the graph's vertices one by one (HugeGraph
///   removes incident edges with each vertex). The REST API has no batch
///   delete, and the global graph-clear endpoint is never used.
#[derive(Debug, Clone)]
pub struct HugeGraphStore {
    client: Client,
    config: HugeGraphConfig,
}

impl HugeGraphStore {
    /// Creates a store for the configured HugeGraph graph. No request is
    /// sent until the first operation.
    ///
    /// # Errors
    ///
    /// [`BackendError`](GraphStoreError::BackendError) if the HTTP client
    /// cannot be built.
    pub fn new(config: HugeGraphConfig) -> Result<Self, GraphStoreError> {
        Ok(Self {
            client: Client::new(&config)?,
            config,
        })
    }

    /// Creates MindGraph's physical schema (property keys, labels and
    /// indexes) if it does not exist yet. Idempotent.
    ///
    /// # Errors
    ///
    /// [`Unsupported`](GraphStoreError::Unsupported) if the HugeGraph graph
    /// already defines one of MindGraph's schema elements differently, or a
    /// backend error.
    pub async fn ensure_schema(&self) -> Result<(), GraphStoreError> {
        let started = Instant::now();
        schema::ensure(&self.client).await?;
        debug!(
            operation = "ensure_schema",
            elapsed_ms = elapsed_ms(started),
            "HugeGraph schema ready"
        );
        Ok(())
    }

    /// The configuration this store was created with.
    #[must_use]
    pub const fn config(&self) -> &HugeGraphConfig {
        &self.config
    }

    /// Loads the given entities of one graph. Missing ids are left out.
    async fn fetch_entities(
        &self,
        graph_id: GraphId,
        ids: &BTreeSet<EntityId>,
    ) -> Result<BTreeMap<EntityId, Entity>, GraphStoreError> {
        let ids: Vec<EntityId> = ids.iter().copied().collect();
        let mut found = BTreeMap::new();
        for chunk in ids.chunks(ID_LOOKUP_CHUNK) {
            let query: Vec<(&str, String)> = chunk
                .iter()
                .map(|id| ("ids", json_string(&codec::vertex_id(graph_id, *id))))
                .collect();
            let page: VertexPage = self.client.get(&["traversers", "vertices"], &query).await?;
            for vertex in &page.vertices {
                let entity = decode_entity(graph_id, vertex)?;
                found.insert(entity.id(), entity);
            }
        }
        Ok(found)
    }

    /// Reads every `mindgraph_entity` vertex matching an indexed filter,
    /// page by page.
    async fn scan_vertices(
        &self,
        filter: &Map<String, Value>,
        cap: Option<usize>,
    ) -> Result<Vec<Vertex>, GraphStoreError> {
        let filter = Value::Object(filter.clone()).to_string();
        let mut vertices = Vec::new();
        // HugeGraph 1.7 repeats the last vertex of a page as the first one
        // of the next page when paging a property-filtered query, so every
        // vertex is kept once, by id.
        let mut seen = BTreeSet::new();
        let mut page = String::new();
        loop {
            let query = [
                ("label", VERTEX_LABEL.to_owned()),
                ("properties", filter.clone()),
                ("page", page),
                ("limit", PAGE_SIZE.to_string()),
            ];
            let result: VertexPage = self.client.get(&["graph", "vertices"], &query).await?;
            vertices.extend(
                result
                    .vertices
                    .into_iter()
                    .filter(|vertex| seen.insert(vertex.id.to_string())),
            );
            if cap.is_some_and(|cap| vertices.len() > cap) {
                return Err(too_many(cap.unwrap_or_default()));
            }
            match result.page {
                Some(next) if !next.is_empty() => page = next,
                _ => return Ok(vertices),
            }
        }
    }

    /// The relationships touching an entity in a direction, by id.
    async fn adjacent(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
        direction: Direction,
    ) -> Result<BTreeMap<RelationshipId, Relationship>, GraphStoreError> {
        let vertex = json_string(&codec::vertex_id(graph_id, entity_id));
        let direction = match direction {
            Direction::Outgoing => "OUT",
            Direction::Incoming => "IN",
            Direction::Both => "BOTH",
        };
        let mut relationships = BTreeMap::new();
        let mut scanned = 0;
        let mut page = String::new();
        loop {
            let query = [
                ("vertex_id", vertex.clone()),
                ("direction", direction.to_owned()),
                ("label", EDGE_LABEL.to_owned()),
                ("page", page),
                ("limit", PAGE_SIZE.to_string()),
            ];
            let result: EdgePage = self.client.get(&["graph", "edges"], &query).await?;
            scanned += result.edges.len();
            if scanned > MAX_SCAN {
                return Err(too_many(MAX_SCAN));
            }
            // BOTH lists a self-loop twice; the map keeps one copy.
            for edge in &result.edges {
                let relationship = decode_relationship(graph_id, edge)?;
                relationships.insert(relationship.id(), relationship);
            }
            match result.page {
                Some(next) if !next.is_empty() => page = next,
                _ => return Ok(relationships),
            }
        }
    }

    /// Which of `ids` already exist as relationships in the graph.
    async fn existing_relationship_ids(
        &self,
        graph_id: GraphId,
        ids: &[RelationshipId],
    ) -> Result<BTreeSet<RelationshipId>, GraphStoreError> {
        let mut existing = BTreeSet::new();
        for chunk in ids.chunks(ID_LOOKUP_CHUNK) {
            let values: Vec<String> = chunk
                .iter()
                .map(|id| json_string(&id.to_string()))
                .collect();
            let mut filter = Map::new();
            filter.insert(PROP_GRAPH_ID.into(), graph_id.to_string().into());
            filter.insert(
                PROP_ID.into(),
                format!("P.within({})", values.join(",")).into(),
            );
            let query = [
                ("label", EDGE_LABEL.to_owned()),
                ("properties", Value::Object(filter).to_string()),
                ("limit", PAGE_SIZE.to_string()),
            ];
            let page: EdgePage = self.client.get(&["graph", "edges"], &query).await?;
            for edge in &page.edges {
                existing.insert(decode_relationship(graph_id, edge)?.id());
            }
        }
        Ok(existing)
    }

    /// Deletes one vertex (and, in HugeGraph, its edges). Returns whether
    /// it existed.
    async fn delete_vertex(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<bool, GraphStoreError> {
        let id = json_string(&codec::vertex_id(graph_id, entity_id));
        match self
            .client
            .delete(
                &["graph", "vertices", &id],
                &[("label", VERTEX_LABEL.to_owned())],
            )
            .await
        {
            Ok(()) => Ok(true),
            // Deleted concurrently: HugeGraph 1.7 answers 400 "No such vertex".
            Err(Failure::Status {
                status: StatusCode::BAD_REQUEST,
                message,
                ..
            }) if message.contains("No such vertex") => Ok(false),
            Err(failure) => Err(failure.into()),
        }
    }
}

#[async_trait]
impl GraphStore for HugeGraphStore {
    async fn ping(&self) -> Result<(), GraphStoreError> {
        let _label: Value = self
            .client
            .get(&["schema", "vertexlabels", VERTEX_LABEL], &[])
            .await?;
        Ok(())
    }

    async fn create_entity(&self, entity: Entity) -> Result<(), GraphStoreError> {
        self.create_entities(vec![entity]).await
    }

    async fn create_entities(&self, entities: Vec<Entity>) -> Result<(), GraphStoreError> {
        let started = Instant::now();
        let Some(graph_id) = validate::batch_graph(&entities, Entity::graph_id, "entities")? else {
            return Ok(());
        };
        validate::unique_ids(&entities, Entity::id, "entity")?;

        let ids = entities.iter().map(Entity::id).collect();
        let existing = self.fetch_entities(graph_id, &ids).await?;
        if let Some(entity) = entities.iter().find(|e| existing.contains_key(&e.id())) {
            return Err(GraphStoreError::Conflict(format!(
                "entity {} already exists in graph {graph_id}",
                entity.id()
            )));
        }

        let vertices = entities
            .iter()
            .map(codec::new_vertex)
            .collect::<Result<Vec<_>, _>>()?;
        for chunk in vertices.chunks(WRITE_BATCH) {
            let _ids: Vec<Value> = self
                .client
                .post(&["graph", "vertices", "batch"], &[], chunk)
                .await?;
        }
        debug!(
            operation = "create_entities",
            %graph_id,
            entity_count = entities.len(),
            elapsed_ms = elapsed_ms(started),
            "HugeGraph write completed"
        );
        Ok(())
    }

    async fn create_relationship(&self, relationship: Relationship) -> Result<(), GraphStoreError> {
        self.create_relationships(vec![relationship]).await
    }

    async fn create_relationships(
        &self,
        relationships: Vec<Relationship>,
    ) -> Result<(), GraphStoreError> {
        let started = Instant::now();
        let Some(graph_id) =
            validate::batch_graph(&relationships, Relationship::graph_id, "relationships")?
        else {
            return Ok(());
        };
        validate::unique_ids(&relationships, Relationship::id, "relationship")?;

        let endpoints = relationships
            .iter()
            .flat_map(|r| [r.source_id(), r.target_id()])
            .collect();
        let found = self.fetch_entities(graph_id, &endpoints).await?;
        for relationship in &relationships {
            for endpoint in [relationship.source_id(), relationship.target_id()] {
                if !found.contains_key(&endpoint) {
                    return Err(GraphStoreError::EntityNotFound {
                        graph_id,
                        entity_id: endpoint,
                    });
                }
            }
        }

        let ids: Vec<RelationshipId> = relationships.iter().map(Relationship::id).collect();
        let existing = self.existing_relationship_ids(graph_id, &ids).await?;
        if let Some(relationship) = relationships.iter().find(|r| existing.contains(&r.id())) {
            return Err(GraphStoreError::Conflict(format!(
                "relationship {} already exists in graph {graph_id}",
                relationship.id()
            )));
        }

        let edges = relationships
            .iter()
            .map(codec::new_edge)
            .collect::<Result<Vec<_>, _>>()?;
        for chunk in edges.chunks(WRITE_BATCH) {
            let _ids: Vec<Value> = self
                .client
                .post(
                    &["graph", "edges", "batch"],
                    &[("check_vertex", "true".to_owned())],
                    chunk,
                )
                .await?;
        }
        debug!(
            operation = "create_relationships",
            %graph_id,
            relationship_count = relationships.len(),
            elapsed_ms = elapsed_ms(started),
            "HugeGraph write completed"
        );
        Ok(())
    }

    async fn get_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<Option<Entity>, GraphStoreError> {
        let mut found = self
            .fetch_entities(graph_id, &BTreeSet::from([entity_id]))
            .await?;
        Ok(found.remove(&entity_id))
    }

    async fn search_entities(
        &self,
        graph_id: GraphId,
        search: &EntitySearch,
    ) -> Result<Vec<Entity>, GraphStoreError> {
        let started = Instant::now();
        let mut filter = Map::new();
        filter.insert(PROP_GRAPH_ID.into(), graph_id.to_string().into());
        if let Some(entity_type) = search.entity_type() {
            filter.insert(PROP_TYPE.into(), entity_type.to_string().into());
        }
        let vertices = self.scan_vertices(&filter, Some(MAX_SCAN)).await?;
        let scanned = vertices.len();
        let mut matches = Vec::new();
        for vertex in &vertices {
            let entity = decode_entity(graph_id, vertex)?;
            if search.matches(&entity) {
                matches.push(entity);
            }
        }
        matches.sort_by_key(Entity::id);
        matches.truncate(search.limit());
        debug!(
            operation = "search_entities",
            %graph_id,
            scanned,
            entity_count = matches.len(),
            elapsed_ms = elapsed_ms(started),
            "HugeGraph read completed"
        );
        Ok(matches)
    }

    async fn get_neighbors(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
        query: &NeighborQuery,
    ) -> Result<Subgraph, GraphStoreError> {
        let started = Instant::now();
        let source =
            self.get_entity(graph_id, entity_id)
                .await?
                .ok_or(GraphStoreError::EntityNotFound {
                    graph_id,
                    entity_id,
                })?;
        let relationships: Vec<Relationship> = self
            .adjacent(graph_id, entity_id, query.direction())
            .await?
            .into_values()
            .filter(|relationship| query.accepts(relationship))
            .take(query.limit())
            .collect();
        let neighbor_ids: BTreeSet<EntityId> = relationships
            .iter()
            .flat_map(|r| [r.source_id(), r.target_id()])
            .filter(|id| *id != entity_id)
            .collect();
        let neighbors = self.fetch_entities(graph_id, &neighbor_ids).await?;
        if let Some(missing) = neighbor_ids.iter().find(|id| !neighbors.contains_key(id)) {
            return Err(GraphStoreError::CorruptData(format!(
                "a relationship of entity {entity_id} points to missing entity {missing}"
            )));
        }
        debug!(
            operation = "get_neighbors",
            %graph_id,
            relationship_count = relationships.len(),
            elapsed_ms = elapsed_ms(started),
            "HugeGraph read completed"
        );
        let entities = std::iter::once(source)
            .chain(neighbors.into_values())
            .collect();
        Ok(Subgraph::new(entities, relationships))
    }

    async fn find_path(
        &self,
        graph_id: GraphId,
        source: EntityId,
        target: EntityId,
        query: &PathQuery,
    ) -> Result<Option<GraphPath>, GraphStoreError> {
        let started = Instant::now();
        let endpoints = self
            .fetch_entities(graph_id, &BTreeSet::from([source, target]))
            .await?;
        for endpoint in [source, target] {
            if !endpoints.contains_key(&endpoint) {
                return Err(GraphStoreError::EntityNotFound {
                    graph_id,
                    entity_id: endpoint,
                });
            }
        }
        if source == target {
            let entity = endpoints.into_values().collect();
            return GraphPath::new(entity, Vec::new()).map(Some);
        }

        // Breadth-first search in MindGraph, one level per hop, exactly as the
        // reference implementation (MemoryGraphStore): relationships are
        // visited in id order and the first discovery of an entity wins, so
        // both backends return the same path and each hop uses the smallest
        // relationship id. HugeGraph's `shortestpath` traverser is not used:
        // it allocates fixed-size records for every level and ran out of heap
        // (OutOfMemoryError) on a 35-entity graph with 2 GiB (M10), which left
        // the server refusing other requests.
        let mut parents: BTreeMap<EntityId, (EntityId, Relationship)> = BTreeMap::new();
        let mut visited = BTreeSet::from([source]);
        let mut frontier = vec![source];
        let mut expanded = 0_usize;
        let mut found = false;
        'search: for _ in 0..query.max_depth() {
            if frontier.is_empty() {
                break;
            }
            let mut next = Vec::new();
            for &current in &frontier {
                expanded += 1;
                if expanded > MAX_PATH_EXPANSIONS {
                    return Err(GraphStoreError::InvalidInput(format!(
                        "the path search explored more than {MAX_PATH_EXPANSIONS} entities; \
                         lower max_depth or choose closer entities"
                    )));
                }
                let adjacent = self.adjacent(graph_id, current, query.direction()).await?;
                for relationship in adjacent.into_values() {
                    let Some(reached) = query.direction().follow(&relationship, current) else {
                        continue;
                    };
                    if visited.insert(reached) {
                        parents.insert(reached, (current, relationship));
                        if reached == target {
                            found = true;
                            break 'search;
                        }
                        next.push(reached);
                    }
                }
            }
            frontier = next;
        }
        if !found {
            return Ok(None);
        }

        let mut path_ids = vec![target];
        let mut relationships = Vec::new();
        let mut cursor = target;
        while let Some((parent, relationship)) = parents.remove(&cursor) {
            relationships.push(relationship);
            path_ids.push(parent);
            cursor = parent;
        }
        path_ids.reverse();
        relationships.reverse();
        let mut loaded = self
            .fetch_entities(graph_id, &path_ids.iter().copied().collect())
            .await?;
        let entities = path_ids
            .iter()
            .map(|id| {
                loaded.remove(id).ok_or_else(|| {
                    GraphStoreError::CorruptData(format!("path entity {id} is missing"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        debug!(
            operation = "find_path",
            %graph_id,
            hops = relationships.len(),
            expanded,
            elapsed_ms = elapsed_ms(started),
            "HugeGraph read completed"
        );
        GraphPath::new(entities, relationships)
            .map(Some)
            .map_err(|err| GraphStoreError::CorruptData(err.to_string()))
    }

    async fn get_subgraph(
        &self,
        graph_id: GraphId,
        entity_ids: &[EntityId],
    ) -> Result<Subgraph, GraphStoreError> {
        let started = Instant::now();
        let requested = validate::subgraph_request(entity_ids)?;
        let found = self.fetch_entities(graph_id, &requested).await?;
        let mut relationships = Vec::new();
        for id in found.keys() {
            let outgoing = self.adjacent(graph_id, *id, Direction::Outgoing).await?;
            relationships.extend(
                outgoing
                    .into_values()
                    .filter(|relationship| found.contains_key(&relationship.target_id())),
            );
        }
        debug!(
            operation = "get_subgraph",
            %graph_id,
            entity_count = found.len(),
            relationship_count = relationships.len(),
            elapsed_ms = elapsed_ms(started),
            "HugeGraph read completed"
        );
        Ok(Subgraph::new(found.into_values().collect(), relationships))
    }

    async fn delete_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<bool, GraphStoreError> {
        if self.get_entity(graph_id, entity_id).await?.is_none() {
            return Ok(false);
        }
        self.delete_vertex(graph_id, entity_id).await
    }

    async fn clear_graph(&self, graph_id: GraphId) -> Result<(), GraphStoreError> {
        let started = Instant::now();
        let mut filter = Map::new();
        filter.insert(PROP_GRAPH_ID.into(), graph_id.to_string().into());
        let vertices = self.scan_vertices(&filter, None).await?;
        let mut deleted = 0_usize;
        for vertex in &vertices {
            let (vertex_graph, entity_id) = codec::parse_vertex_id(&vertex.id)?;
            // Never delete outside the requested graph, whatever the index says.
            if vertex_graph != graph_id {
                return Err(GraphStoreError::CorruptData(format!(
                    "index returned vertex {} for graph {graph_id}",
                    vertex.id
                )));
            }
            if self.delete_vertex(graph_id, entity_id).await? {
                deleted += 1;
            }
        }
        debug!(
            operation = "clear_graph",
            %graph_id,
            entity_count = deleted,
            elapsed_ms = elapsed_ms(started),
            "HugeGraph graph cleared"
        );
        Ok(())
    }
}

/// Decodes a vertex and checks it belongs to `graph_id`.
fn decode_entity(graph_id: GraphId, vertex: &Vertex) -> Result<Entity, GraphStoreError> {
    let entity = codec::entity_from_vertex(vertex)?;
    if entity.graph_id() == graph_id {
        Ok(entity)
    } else {
        Err(GraphStoreError::CorruptData(format!(
            "vertex {} was returned for graph {graph_id}",
            vertex.id
        )))
    }
}

/// Decodes an edge and checks it belongs to `graph_id`.
fn decode_relationship(
    graph_id: GraphId,
    edge: &dto::Edge,
) -> Result<Relationship, GraphStoreError> {
    let relationship = codec::relationship_from_edge(edge)?;
    if relationship.graph_id() == graph_id {
        Ok(relationship)
    } else {
        Err(GraphStoreError::CorruptData(format!(
            "relationship {} was returned for graph {graph_id}",
            relationship.id()
        )))
    }
}

/// HugeGraph expects ids in query strings and paths as JSON values.
fn json_string(value: &str) -> String {
    Value::from(value).to_string()
}

fn too_many(cap: usize) -> GraphStoreError {
    GraphStoreError::Unsupported(format!(
        "the operation would scan more than {cap} elements in HugeGraph; narrow the query"
    ))
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
