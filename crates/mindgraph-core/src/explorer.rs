//! Graph Explorer use cases (M6): read-only, bounded retrieval of one
//! graph's entities, neighbourhoods, shortest paths and subgraphs.
//!
//! ```text
//! mindgraph-api ─► GraphExplorerService ─► GraphStore (Memory | HugeGraph)
//!                     │ checks the GraphProject (workspace scoped)
//!                     │ decodes exact-match filters with the active ontology
//!                     └ maps GraphStore errors to ApplicationError
//! ```
//!
//! Every operation first checks that the graph exists in the workspace.
//! Everything else is scoped by the `GraphId` the GraphStore receives, so an
//! entity of another graph is simply missing. Nothing here writes, and
//! nothing is unbounded: the GraphStore query types cap results and depth.
//! Logs carry ids, operations, counts and durations, never property values.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Instant;

use mindgraph_domain::{
    Entity, EntityId, GraphId, Ontology, PropertyKey, PropertyType, PropertyValue, TypeName,
    WorkspaceId,
};
use mindgraph_graph::{
    EntitySearch, GraphPath, GraphStore, GraphStoreError, MAX_SUBGRAPH_ENTITIES, NeighborQuery,
    PathQuery, Subgraph,
};
use serde_json::Value;
use tracing::info;

use crate::{ApplicationError, MetadataStore};

/// Largest number of exact-match property filters in one search.
pub const MAX_SEARCH_FILTERS: usize = 20;

/// An exact-match entity search as clients express it: an optional entity
/// type and property values in JSON, typed through the active ontology.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EntitySearchRequest {
    /// Only entities of this type.
    pub entity_type: Option<String>,
    /// Property name to exact value. Integers may also be given as decimal
    /// strings (for values beyond JavaScript's safe range), datetimes as RFC
    /// 3339 strings.
    pub property_filters: BTreeMap<String, Value>,
    /// At most this many results (`1..=EntitySearch::MAX_LIMIT`); the
    /// GraphStore default when absent.
    pub limit: Option<usize>,
}

/// Read-only graph retrieval for the Graph Explorer (and, later, other
/// readers such as GraphRAG and MCP).
#[derive(Clone)]
pub struct GraphExplorerService {
    metadata: Arc<dyn MetadataStore>,
    graph: Arc<dyn GraphStore>,
    workspace_id: WorkspaceId,
}

impl std::fmt::Debug for GraphExplorerService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphExplorerService")
            .field("workspace_id", &self.workspace_id)
            .finish_non_exhaustive()
    }
}

fn graph_failure(error: GraphStoreError) -> ApplicationError {
    match error {
        GraphStoreError::EntityNotFound { entity_id, .. } => {
            ApplicationError::EntityNotFound(entity_id)
        }
        // Query bounds; the messages name limits, never stored data.
        GraphStoreError::InvalidInput(message) => ApplicationError::InvalidRequest(message),
        GraphStoreError::BackendUnavailable { .. } => ApplicationError::unavailable(error),
        other => ApplicationError::internal(other),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl GraphExplorerService {
    /// A service over `graph` for the graphs of `workspace_id`.
    #[must_use]
    pub fn new(
        metadata: Arc<dyn MetadataStore>,
        graph: Arc<dyn GraphStore>,
        workspace_id: WorkspaceId,
    ) -> Self {
        Self {
            metadata,
            graph,
            workspace_id,
        }
    }

    /// The graph must exist in this workspace.
    async fn ensure_graph(&self, graph_id: GraphId) -> Result<(), ApplicationError> {
        self.metadata
            .get_graph(graph_id)
            .await?
            .filter(|graph| graph.workspace_id() == self.workspace_id)
            .map(|_| ())
            .ok_or(ApplicationError::GraphNotFound(graph_id))
    }

    /// One entity of the graph.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ApplicationError::EntityNotFound`]
    /// (also for an entity of another graph), or a backend failure.
    pub async fn get_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<Entity, ApplicationError> {
        self.ensure_graph(graph_id).await?;
        let started = Instant::now();
        let entity = self
            .graph
            .get_entity(graph_id, entity_id)
            .await
            .map_err(graph_failure)?
            .ok_or(ApplicationError::EntityNotFound(entity_id))?;
        info!(
            graph_id = %graph_id,
            operation = "get_entity",
            duration_ms = elapsed_ms(started),
            "graph read"
        );
        Ok(entity)
    }

    /// Entities matching every filter exactly, ordered by id. This is exact
    /// matching on typed property values, not text search.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ApplicationError::InvalidRequest`]
    /// for an unknown entity type or property, a value of the wrong type, too
    /// many filters or a limit out of range, or a backend failure.
    pub async fn search_entities(
        &self,
        graph_id: GraphId,
        request: &EntitySearchRequest,
    ) -> Result<Vec<Entity>, ApplicationError> {
        self.ensure_graph(graph_id).await?;
        let ontology = self
            .metadata
            .get_active_ontology(graph_id)
            .await?
            .ok_or(ApplicationError::GraphNotFound(graph_id))?;
        let search = build_search(&ontology, request)?;
        let started = Instant::now();
        let entities = self
            .graph
            .search_entities(graph_id, &search)
            .await
            .map_err(graph_failure)?;
        info!(
            graph_id = %graph_id,
            operation = "search_entities",
            filter_count = request.property_filters.len(),
            result_count = entities.len(),
            duration_ms = elapsed_ms(started),
            "graph read"
        );
        Ok(entities)
    }

    /// The entity, its matching relationships (at most `query.limit()`) and
    /// the entities at their other ends.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ApplicationError::EntityNotFound`],
    /// or a backend failure.
    pub async fn neighbors(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
        query: &NeighborQuery,
    ) -> Result<Subgraph, ApplicationError> {
        self.ensure_graph(graph_id).await?;
        let started = Instant::now();
        let subgraph = self
            .graph
            .get_neighbors(graph_id, entity_id, query)
            .await
            .map_err(graph_failure)?;
        info!(
            graph_id = %graph_id,
            operation = "neighbors",
            entity_count = subgraph.entities().len(),
            relationship_count = subgraph.relationships().len(),
            duration_ms = elapsed_ms(started),
            "graph read"
        );
        Ok(subgraph)
    }

    /// A shortest path (fewest hops) within `query.max_depth()` hops, or
    /// `None`. A path from an entity to itself has zero hops.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ApplicationError::EntityNotFound`]
    /// for a missing endpoint, or a backend failure.
    pub async fn find_path(
        &self,
        graph_id: GraphId,
        source: EntityId,
        target: EntityId,
        query: &PathQuery,
    ) -> Result<Option<GraphPath>, ApplicationError> {
        self.ensure_graph(graph_id).await?;
        let started = Instant::now();
        let path = self
            .graph
            .find_path(graph_id, source, target, query)
            .await
            .map_err(graph_failure)?;
        info!(
            graph_id = %graph_id,
            operation = "find_path",
            max_depth = query.max_depth(),
            found = path.is_some(),
            hops = path.as_ref().map_or(0, GraphPath::hops),
            duration_ms = elapsed_ms(started),
            "graph read"
        );
        Ok(path)
    }

    /// The requested entities that exist in the graph and the relationships
    /// between them. Missing ids are ignored.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ApplicationError::InvalidRequest`]
    /// for more than [`MAX_SUBGRAPH_ENTITIES`] distinct ids, or a backend
    /// failure.
    pub async fn subgraph(
        &self,
        graph_id: GraphId,
        entity_ids: &[EntityId],
    ) -> Result<Subgraph, ApplicationError> {
        let distinct: BTreeSet<_> = entity_ids.iter().collect();
        if distinct.len() > MAX_SUBGRAPH_ENTITIES {
            return Err(ApplicationError::InvalidRequest(format!(
                "a subgraph request may name at most {MAX_SUBGRAPH_ENTITIES} entities"
            )));
        }
        self.ensure_graph(graph_id).await?;
        let started = Instant::now();
        let subgraph = self
            .graph
            .get_subgraph(graph_id, entity_ids)
            .await
            .map_err(graph_failure)?;
        info!(
            graph_id = %graph_id,
            operation = "subgraph",
            requested = distinct.len(),
            entity_count = subgraph.entities().len(),
            relationship_count = subgraph.relationships().len(),
            duration_ms = elapsed_ms(started),
            "graph read"
        );
        Ok(subgraph)
    }
}

/// The GraphStore search for a request, typing each filter value through the
/// active ontology: the entity type's schema, or, without a type, the one
/// type every entity type that defines the property agrees on.
fn build_search(
    ontology: &Ontology,
    request: &EntitySearchRequest,
) -> Result<EntitySearch, ApplicationError> {
    if request.property_filters.len() > MAX_SEARCH_FILTERS {
        return Err(ApplicationError::InvalidRequest(format!(
            "at most {MAX_SEARCH_FILTERS} property filters are allowed"
        )));
    }
    let entity_type = match request.entity_type.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(name) => {
            let name = TypeName::new(name)
                .map_err(|_| ApplicationError::InvalidRequest("invalid entity type name".into()))?;
            if ontology.entity_type(name.as_str()).is_none() {
                return Err(ApplicationError::InvalidRequest(format!(
                    "entity type `{name}` is not in the graph's ontology"
                )));
            }
            Some(name)
        }
    };
    let mut search = EntitySearch::new();
    if let Some(limit) = request.limit {
        search = search.with_limit(limit).map_err(graph_failure)?;
    }
    for (raw_key, raw_value) in &request.property_filters {
        let key = PropertyKey::new(raw_key.trim())
            .map_err(|_| ApplicationError::InvalidRequest("invalid property name".into()))?;
        let expected = property_type(ontology, entity_type.as_ref(), &key)?;
        let value = typed_value(&key, raw_value, expected)?;
        search = search.with_property(key, value);
    }
    if let Some(entity_type) = entity_type {
        search = search.with_entity_type(entity_type);
    }
    Ok(search)
}

fn property_type(
    ontology: &Ontology,
    entity_type: Option<&TypeName>,
    key: &PropertyKey,
) -> Result<PropertyType, ApplicationError> {
    if let Some(entity_type) = entity_type {
        return ontology
            .entity_type(entity_type.as_str())
            .and_then(|t| t.properties_schema().get(key.as_str()))
            .map(|definition| definition.property_type)
            .ok_or_else(|| {
                ApplicationError::InvalidRequest(format!(
                    "property `{key}` is not defined for `{entity_type}`"
                ))
            });
    }
    let types: HashSet<PropertyType> = ontology
        .entity_types()
        .iter()
        .filter_map(|t| t.properties_schema().get(key.as_str()))
        .map(|definition| definition.property_type)
        .collect();
    let mut types = types.into_iter();
    match (types.next(), types.next()) {
        (Some(only), None) => Ok(only),
        (None, _) => Err(ApplicationError::InvalidRequest(format!(
            "property `{key}` is not defined in the graph's ontology"
        ))),
        (Some(_), Some(_)) => Err(ApplicationError::InvalidRequest(format!(
            "property `{key}` has different types in different entity types; set entity_type"
        ))),
    }
}

/// The filter value as `expected`. Integers also accept decimal strings, so
/// clients can send values beyond JavaScript's safe range exactly.
fn typed_value(
    key: &PropertyKey,
    value: &Value,
    expected: PropertyType,
) -> Result<PropertyValue, ApplicationError> {
    if value.is_null() {
        return Err(ApplicationError::InvalidRequest(format!(
            "the value of filter `{key}` must not be null"
        )));
    }
    if expected == PropertyType::Integer
        && let Value::String(text) = value
    {
        return text
            .trim()
            .parse::<i64>()
            .map(PropertyValue::Integer)
            .map_err(|_| {
                ApplicationError::InvalidRequest(format!(
                    "the value of filter `{key}` must be an integer"
                ))
            });
    }
    PropertyValue::from_json(key.as_str(), value, expected)
        .map_err(|err| ApplicationError::InvalidRequest(err.to_string()))
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::{
        EntityType, EntityTypeId, OntologyVersion, PropertiesSchema, PropertyDefinition,
    };
    use serde_json::json;

    use super::*;

    fn property(name: &str, property_type: PropertyType) -> PropertyDefinition {
        PropertyDefinition {
            key: PropertyKey::new(name).unwrap(),
            property_type,
            required: false,
        }
    }

    fn ontology() -> Ontology {
        let graph_id = GraphId::new();
        let entity = |name: &str, properties: Vec<PropertyDefinition>| {
            EntityType::new(
                EntityTypeId::new(),
                graph_id,
                TypeName::new(name).unwrap(),
                PropertiesSchema::new(properties).unwrap(),
            )
        };
        Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![
                entity(
                    "Customer",
                    vec![
                        property("customer_id", PropertyType::String),
                        property("age", PropertyType::Integer),
                        property("code", PropertyType::String),
                    ],
                ),
                entity(
                    "Account",
                    vec![
                        property("opened_at", PropertyType::DateTime),
                        property("code", PropertyType::Integer),
                    ],
                ),
            ],
            vec![],
        )
        .unwrap()
    }

    fn request(entity_type: Option<&str>, filters: Value) -> EntitySearchRequest {
        EntitySearchRequest {
            entity_type: entity_type.map(str::to_owned),
            property_filters: serde_json::from_value(filters).unwrap(),
            limit: None,
        }
    }

    #[test]
    fn filters_are_typed_by_the_ontology() {
        let search = build_search(
            &ontology(),
            &request(
                Some("Customer"),
                json!({"customer_id": "C001", "age": "9007199254740993"}),
            ),
        )
        .unwrap();
        let filters = search.property_filters();
        assert_eq!(
            filters.get(&PropertyKey::new("age").unwrap()),
            Some(&PropertyValue::Integer(9_007_199_254_740_993)),
            "big integers keep their exact value"
        );
        assert_eq!(
            filters.get(&PropertyKey::new("customer_id").unwrap()),
            Some(&PropertyValue::String("C001".into()))
        );

        // Without a type, a property with one type across the ontology works.
        let search = build_search(
            &ontology(),
            &request(None, json!({"opened_at": "2024-02-11T16:30:00+07:00"})),
        )
        .unwrap();
        assert!(matches!(
            search.property_filters().values().next(),
            Some(PropertyValue::DateTime(at)) if at.to_rfc3339() == "2024-02-11T09:30:00+00:00"
        ));
    }

    #[test]
    fn invalid_filters_are_client_errors() {
        for (entity_type, filters) in [
            (Some("Device"), json!({})),
            (Some("Customer"), json!({"unknown": "x"})),
            (Some("Customer"), json!({"age": "twelve"})),
            (Some("Customer"), json!({"age": 1.5})),
            (Some("Customer"), json!({"customer_id": null})),
            (None, json!({"code": "1"})),
            (None, json!({"missing": 1})),
            (Some("bad name"), json!({})),
        ] {
            let result = build_search(&ontology(), &request(entity_type, filters.clone()));
            assert!(
                matches!(result, Err(ApplicationError::InvalidRequest(_))),
                "{entity_type:?} {filters}"
            );
        }
        let too_many: BTreeMap<String, Value> = (0..=MAX_SEARCH_FILTERS)
            .map(|i| (format!("p{i}"), json!(1)))
            .collect();
        let result = build_search(
            &ontology(),
            &EntitySearchRequest {
                property_filters: too_many,
                ..EntitySearchRequest::default()
            },
        );
        assert!(matches!(result, Err(ApplicationError::InvalidRequest(_))));
    }
}
