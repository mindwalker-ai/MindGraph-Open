//! Graph Explorer endpoints (M6): read-only, bounded graph retrieval.
//!
//! Handlers parse and validate the request, call `GraphExplorerService` and
//! render explicit DTOs. They contain no traversal logic and never see the
//! graph backend. Entity and relationship ids are the domain UUIDs, never
//! a backend's physical ids.
//!
//! Property values are rendered as plain JSON next to their `property_types`
//! (`string`, `integer`, `float`, `boolean`, `datetime`, `string_array`), so
//! clients keep datetimes apart from strings. Integers beyond JavaScript's
//! safe range (±(2^53 − 1)) are rendered as decimal strings rather than
//! silently losing precision; their type stays `integer`.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use mindgraph_core::{EntitySearchRequest, GraphExplorerService};
use mindgraph_domain::{
    Entity, EntityId, GraphId, Properties, PropertyValue, Relationship, TypeName,
};
use mindgraph_graph::{
    Direction, EntitySearch, GraphPath, MAX_SUBGRAPH_ENTITIES, NeighborQuery, PathQuery, Subgraph,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ApiError;
use crate::routes::{AppState, graph_id};

/// The largest integer JavaScript represents exactly.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

// --- DTOs ------------------------------------------------------------------------------

/// A property value as JSON, keeping integers beyond JavaScript's safe range
/// exact as decimal strings.
fn property_json(value: &PropertyValue) -> Value {
    match value {
        PropertyValue::Integer(integer) if integer.unsigned_abs() > MAX_SAFE_INTEGER => {
            Value::String(integer.to_string())
        }
        other => other.to_json(),
    }
}

type RenderedProperties = (BTreeMap<String, Value>, BTreeMap<String, &'static str>);

fn render_properties(properties: &Properties) -> RenderedProperties {
    properties
        .iter()
        .map(|(key, value)| {
            (
                (key.to_string(), property_json(value)),
                (key.to_string(), value.property_type().as_str()),
            )
        })
        .unzip()
}

/// An entity: its type and typed properties.
#[derive(Debug, Serialize)]
pub(crate) struct EntityDto {
    id: String,
    graph_id: String,
    entity_type: String,
    properties: BTreeMap<String, Value>,
    property_types: BTreeMap<String, &'static str>,
}

impl From<&Entity> for EntityDto {
    fn from(entity: &Entity) -> Self {
        let (properties, property_types) = render_properties(entity.properties());
        Self {
            id: entity.id().to_string(),
            graph_id: entity.graph_id().to_string(),
            entity_type: entity.entity_type().to_string(),
            properties,
            property_types,
        }
    }
}

/// A directed relationship between two entities of the same graph.
#[derive(Debug, Serialize)]
pub(crate) struct RelationshipDto {
    id: String,
    graph_id: String,
    relationship_type: String,
    source_id: String,
    target_id: String,
    properties: BTreeMap<String, Value>,
    property_types: BTreeMap<String, &'static str>,
}

impl From<&Relationship> for RelationshipDto {
    fn from(relationship: &Relationship) -> Self {
        let (properties, property_types) = render_properties(relationship.properties());
        Self {
            id: relationship.id().to_string(),
            graph_id: relationship.graph_id().to_string(),
            relationship_type: relationship.relationship_type().to_string(),
            source_id: relationship.source_id().to_string(),
            target_id: relationship.target_id().to_string(),
            properties,
            property_types,
        }
    }
}

/// Entities and the relationships among them, ordered by id.
#[derive(Debug, Serialize)]
pub(crate) struct SubgraphDto {
    entities: Vec<EntityDto>,
    relationships: Vec<RelationshipDto>,
}

impl From<&Subgraph> for SubgraphDto {
    fn from(subgraph: &Subgraph) -> Self {
        Self {
            entities: subgraph.entities().iter().map(EntityDto::from).collect(),
            relationships: subgraph
                .relationships()
                .iter()
                .map(RelationshipDto::from)
                .collect(),
        }
    }
}

/// A shortest path in order from source to target, or `found: false`.
#[derive(Debug, Serialize)]
pub(crate) struct PathDto {
    found: bool,
    hops: usize,
    entities: Vec<EntityDto>,
    relationships: Vec<RelationshipDto>,
}

impl From<Option<&GraphPath>> for PathDto {
    fn from(path: Option<&GraphPath>) -> Self {
        path.map_or(
            Self {
                found: false,
                hops: 0,
                entities: Vec::new(),
                relationships: Vec::new(),
            },
            |path| Self {
                found: true,
                hops: path.hops(),
                entities: path.entities().iter().map(EntityDto::from).collect(),
                relationships: path
                    .relationships()
                    .iter()
                    .map(RelationshipDto::from)
                    .collect(),
            },
        )
    }
}

/// `POST /api/v1/graphs/{graph_id}/entities/search`: exact matching.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchRequest {
    #[serde(default)]
    entity_type: Option<String>,
    #[serde(default)]
    property_filters: BTreeMap<String, Value>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchResponse {
    data: Vec<EntityDto>,
    count: usize,
    limit: usize,
}

/// `GET .../entities/{entity_id}/neighbors` query.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NeighborsParams {
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    relationship_type: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// `POST /api/v1/graphs/{graph_id}/paths`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PathRequest {
    source_id: String,
    target_id: String,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    max_depth: Option<usize>,
}

/// `POST /api/v1/graphs/{graph_id}/subgraph`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubgraphRequest {
    entity_ids: Vec<String>,
}

// --- parsing -------------------------------------------------------------------------------

fn explorer(state: &AppState) -> Result<&GraphExplorerService, ApiError> {
    state.graph_explorer.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "SERVICE_UNAVAILABLE",
            "The graph explorer is not available",
        )
    })
}

fn invalid_query(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "INVALID_QUERY", message)
}

fn invalid_request(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "INVALID_REQUEST", message)
}

fn entity_id(raw: &str) -> Result<EntityId, ApiError> {
    raw.trim()
        .parse()
        .map_err(|_| ApiError::invalid_id("INVALID_ENTITY_ID", "entity id"))
}

fn entity_path(
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<(GraphId, EntityId), ApiError> {
    let Path((graph, entity)) = path.map_err(|_| ApiError::invalid_graph_id())?;
    let graph = graph.parse().map_err(|_| ApiError::invalid_graph_id())?;
    Ok((graph, entity_id(&entity)?))
}

/// `outgoing`, `incoming` or `both` (the default).
fn direction(raw: Option<&str>) -> Option<Direction> {
    match raw.map(str::trim) {
        None | Some("" | "both") => Some(Direction::Both),
        Some("outgoing") => Some(Direction::Outgoing),
        Some("incoming") => Some(Direction::Incoming),
        Some(_) => None,
    }
}

// --- handlers ------------------------------------------------------------------------------

/// `POST /api/v1/graphs/{graph_id}/entities/search`
pub(crate) async fn search_entities(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<SearchRequest>, JsonRejection>,
) -> Result<Json<SearchResponse>, ApiError> {
    let graph_id = graph_id(path)?;
    let Json(request) = body?;
    let limit = request.limit.unwrap_or(EntitySearch::DEFAULT_LIMIT);
    let entities = explorer(&state)?
        .search_entities(
            graph_id,
            &EntitySearchRequest {
                entity_type: request.entity_type,
                property_filters: request.property_filters,
                limit: Some(limit),
            },
        )
        .await?;
    Ok(Json(SearchResponse {
        count: entities.len(),
        data: entities.iter().map(EntityDto::from).collect(),
        limit,
    }))
}

/// `GET /api/v1/graphs/{graph_id}/entities/{entity_id}`
pub(crate) async fn get_entity(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<Json<EntityDto>, ApiError> {
    let (graph_id, entity_id) = entity_path(path)?;
    let entity = explorer(&state)?.get_entity(graph_id, entity_id).await?;
    Ok(Json(EntityDto::from(&entity)))
}

/// `GET /api/v1/graphs/{graph_id}/entities/{entity_id}/neighbors`
pub(crate) async fn get_neighbors(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
    params: Result<Query<NeighborsParams>, QueryRejection>,
) -> Result<Json<SubgraphDto>, ApiError> {
    let (graph_id, entity_id) = entity_path(path)?;
    let Query(params) = params?;
    let direction = direction(params.direction.as_deref())
        .ok_or_else(|| invalid_query("direction must be outgoing, incoming or both"))?;
    let mut query = NeighborQuery::new(direction)
        .with_limit(params.limit.unwrap_or(NeighborQuery::DEFAULT_LIMIT))
        .map_err(|_| {
            invalid_query(format!(
                "limit must be between 1 and {}",
                NeighborQuery::MAX_LIMIT
            ))
        })?;
    if let Some(raw) = params.relationship_type.as_deref().map(str::trim)
        && !raw.is_empty()
    {
        let relationship_type =
            TypeName::new(raw).map_err(|_| invalid_query("invalid relationship_type"))?;
        query = query.with_relationship_type(relationship_type);
    }
    let subgraph = explorer(&state)?
        .neighbors(graph_id, entity_id, &query)
        .await?;
    Ok(Json(SubgraphDto::from(&subgraph)))
}

/// `POST /api/v1/graphs/{graph_id}/paths`
pub(crate) async fn find_path(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<PathRequest>, JsonRejection>,
) -> Result<Json<PathDto>, ApiError> {
    let graph_id = graph_id(path)?;
    let Json(request) = body?;
    let source = entity_id(&request.source_id)?;
    let target = entity_id(&request.target_id)?;
    let direction = direction(request.direction.as_deref())
        .ok_or_else(|| invalid_request("direction must be outgoing, incoming or both"))?;
    let query = PathQuery::new(direction)
        .with_max_depth(request.max_depth.unwrap_or(PathQuery::DEFAULT_MAX_DEPTH))
        .map_err(|_| {
            invalid_request(format!(
                "max_depth must be between 1 and {}",
                PathQuery::MAX_DEPTH
            ))
        })?;
    let path = explorer(&state)?
        .find_path(graph_id, source, target, &query)
        .await?;
    Ok(Json(PathDto::from(path.as_ref())))
}

/// `POST /api/v1/graphs/{graph_id}/subgraph`
pub(crate) async fn get_subgraph(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<SubgraphRequest>, JsonRejection>,
) -> Result<Json<SubgraphDto>, ApiError> {
    let graph_id = graph_id(path)?;
    let Json(request) = body?;
    if request.entity_ids.len() > MAX_SUBGRAPH_ENTITIES {
        return Err(invalid_request(format!(
            "entity_ids may name at most {MAX_SUBGRAPH_ENTITIES} entities"
        )));
    }
    let ids = request
        .entity_ids
        .iter()
        .map(|raw| entity_id(raw))
        .collect::<Result<Vec<_>, _>>()?;
    let subgraph = explorer(&state)?.subgraph(graph_id, &ids).await?;
    Ok(Json(SubgraphDto::from(&subgraph)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_beyond_javascript_precision_are_strings() {
        let big = 9_007_199_254_740_993_i64;
        assert_eq!(
            property_json(&PropertyValue::Integer(big)),
            Value::String(big.to_string())
        );
        assert_eq!(
            property_json(&PropertyValue::Integer(-big)),
            Value::String((-big).to_string())
        );
        assert_eq!(
            property_json(&PropertyValue::Integer(9_007_199_254_740_991)),
            serde_json::json!(9_007_199_254_740_991_i64)
        );
        assert_eq!(
            property_json(&PropertyValue::Integer(i64::MIN)),
            Value::String(i64::MIN.to_string())
        );
    }

    #[test]
    fn directions_parse_strictly() {
        assert_eq!(direction(None), Some(Direction::Both));
        assert_eq!(direction(Some("incoming")), Some(Direction::Incoming));
        assert_eq!(direction(Some("sideways")), None);
    }
}
