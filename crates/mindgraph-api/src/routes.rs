//! Router, application state and handlers.

use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::get;
use axum::{Json, Router, middleware};
use mindgraph_core::{CreateGraphProject, GraphProjectService};
use mindgraph_domain::GraphId;
use serde::Serialize;

use crate::SERVICE_NAME;
use crate::dto::{
    CreateGraphRequest, GraphListResponse, GraphResponse, ListGraphsQuery, OntologyResponse,
};
use crate::error::ApiError;
use crate::request_context::request_context;

/// Shared state of the HTTP layer: the application services it calls.
#[derive(Debug, Clone)]
pub struct AppState {
    graph_projects: Arc<GraphProjectService>,
}

impl AppState {
    /// Wraps the application services.
    #[must_use]
    pub fn new(graph_projects: GraphProjectService) -> Self {
        Self {
            graph_projects: Arc::new(graph_projects),
        }
    }
}

/// Builds the application router.
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/graphs", get(list_graphs).post(create_graph))
        .route("/graphs/{graph_id}", get(get_graph))
        .route("/graphs/{graph_id}/ontology", get(get_ontology));
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .nest("/api/v1", api)
        .fallback(route_not_found)
        .with_state(state)
        .layer(middleware::from_fn(request_context))
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

/// Liveness: answers as long as the process serves requests. Never touches
/// the database.
async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: SERVICE_NAME,
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Debug, Serialize)]
struct Ready {
    status: &'static str,
}

/// Readiness: checks that PostgreSQL is reachable.
async fn ready(State(state): State<AppState>) -> Result<Json<Ready>, ApiError> {
    state
        .graph_projects
        .check_ready()
        .await
        .map_err(|_| ApiError::not_ready())?;
    Ok(Json(Ready { status: "ready" }))
}

async fn route_not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "ROUTE_NOT_FOUND",
        "No route matches this path",
    )
}

fn graph_id(path: Result<Path<String>, PathRejection>) -> Result<GraphId, ApiError> {
    let Path(raw) = path?;
    raw.parse().map_err(|_| ApiError::invalid_graph_id())
}

async fn create_graph(
    State(state): State<AppState>,
    body: Result<Json<CreateGraphRequest>, JsonRejection>,
) -> Result<(StatusCode, HeaderMap, Json<GraphResponse>), ApiError> {
    let Json(request) = body?;
    let graph = state
        .graph_projects
        .create_graph(CreateGraphProject {
            name: request.name,
            description: request.description,
        })
        .await?;
    let mut headers = HeaderMap::new();
    if let Ok(location) = HeaderValue::from_str(&format!("/api/v1/graphs/{}", graph.id())) {
        headers.insert(header::LOCATION, location);
    }
    Ok((
        StatusCode::CREATED,
        headers,
        Json(GraphResponse::from(&graph)),
    ))
}

async fn list_graphs(
    State(state): State<AppState>,
    query: Result<Query<ListGraphsQuery>, QueryRejection>,
) -> Result<Json<GraphListResponse>, ApiError> {
    let Query(query) = query?;
    let graphs = state.graph_projects.list_graphs(query.limit).await?;
    Ok(Json(GraphListResponse::from(graphs.as_slice())))
}

async fn get_graph(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<GraphResponse>, ApiError> {
    let graph = state.graph_projects.get_graph(graph_id(path)?).await?;
    Ok(Json(GraphResponse::from(&graph)))
}

async fn get_ontology(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<OntologyResponse>, ApiError> {
    let ontology = state
        .graph_projects
        .get_active_ontology(graph_id(path)?)
        .await?;
    Ok(Json(OntologyResponse::from(&ontology)))
}
