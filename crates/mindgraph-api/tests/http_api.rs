//! HTTP API tests against the real router with an in-memory metadata store.
//! No PostgreSQL, network or Docker needed.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use mindgraph_api::{AppState, router};
use mindgraph_core::{GraphProjectService, MemoryMetadataStore, MetadataStore, MetadataStoreError};
use mindgraph_domain::{GraphId, GraphProject, Ontology, OntologyVersion, Workspace, WorkspaceId};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn app_with(store: Arc<dyn MetadataStore>) -> Router {
    router(AppState::new(
        GraphProjectService::bootstrap(store).await.unwrap(),
    ))
}

async fn app() -> Router {
    app_with(Arc::new(MemoryMetadataStore::new())).await
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Reply {
        status,
        headers,
        body,
    }
}

async fn get(app: &Router, uri: &str) -> Reply {
    send(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

async fn post_json(app: &Router, uri: &str, body: Value) -> Reply {
    let request = Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    send(app, request).await
}

async fn create(app: &Router, name: &str) -> Value {
    let reply = post_json(app, "/api/v1/graphs", json!({ "name": name })).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body
}

/// Asserts the ADR-006 envelope and that its request id matches the header.
fn assert_error(reply: &Reply, status: StatusCode, code: &str) {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert_eq!(reply.body["error"]["code"], code, "{}", reply.body);
    assert!(reply.body["error"]["message"].is_string());
    let header_id = reply.headers["x-request-id"].to_str().unwrap();
    assert_eq!(reply.body["error"]["request_id"], header_id);
    assert_eq!(reply.headers[header::CONTENT_TYPE], "application/json");
}

#[tokio::test]
async fn health_and_ready_report_ok() {
    let app = app().await;
    let health = get(&app, "/health").await;
    assert_eq!(health.status, StatusCode::OK);
    assert_eq!(health.body["status"], "ok");
    assert_eq!(health.body["service"], "mindgraph-core");
    let ready = get(&app, "/ready").await;
    assert_eq!(ready.status, StatusCode::OK);
    assert_eq!(ready.body, json!({ "status": "ready" }));
}

#[tokio::test]
async fn create_graph_returns_201_with_the_project() {
    let app = app().await;
    let reply = post_json(
        &app,
        "/api/v1/graphs",
        json!({ "name": "Fraud Investigation", "description": "Transaction investigation graph" }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let body = &reply.body;
    let id = body["id"].as_str().unwrap();
    assert!(id.parse::<GraphId>().is_ok());
    assert_eq!(
        reply.headers[header::LOCATION],
        format!("/api/v1/graphs/{id}")
    );
    assert_eq!(body["name"], "Fraud Investigation");
    assert_eq!(body["description"], "Transaction investigation graph");
    assert_eq!(body["status"], "draft");
    assert!(
        body["workspace_id"]
            .as_str()
            .unwrap()
            .parse::<WorkspaceId>()
            .is_ok()
    );
    assert_eq!(body["created_at"], body["updated_at"]);
    let keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "created_at",
            "description",
            "id",
            "name",
            "status",
            "updated_at",
            "workspace_id"
        ],
        "no internal fields leak"
    );
}

#[tokio::test]
async fn description_is_optional() {
    let app = app().await;
    let body = create(&app, "Cyber").await;
    assert_eq!(body["description"], Value::Null);
}

#[tokio::test]
async fn invalid_names_are_422() {
    let app = app().await;
    for name in ["", "   ", &"x".repeat(121)] {
        let reply = post_json(&app, "/api/v1/graphs", json!({ "name": name })).await;
        assert_error(
            &reply,
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_FAILED",
        );
    }
}

#[tokio::test]
async fn malformed_requests_are_400_or_415() {
    let app = app().await;
    let syntax = send(
        &app,
        Request::post("/api/v1/graphs")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{not json"))
            .unwrap(),
    )
    .await;
    assert_error(&syntax, StatusCode::BAD_REQUEST, "INVALID_JSON");
    let missing_name = post_json(&app, "/api/v1/graphs", json!({ "description": "x" })).await;
    assert_error(&missing_name, StatusCode::BAD_REQUEST, "INVALID_JSON");
    let no_content_type = send(
        &app,
        Request::post("/api/v1/graphs")
            .body(Body::from(r#"{"name":"x"}"#))
            .unwrap(),
    )
    .await;
    assert_error(
        &no_content_type,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "UNSUPPORTED_MEDIA_TYPE",
    );
}

#[tokio::test]
async fn list_graphs_is_newest_first_and_limited() {
    let app = app().await;
    for name in ["first", "second", "third"] {
        create(&app, name).await;
    }
    let all = get(&app, "/api/v1/graphs").await;
    assert_eq!(all.status, StatusCode::OK);
    let names: Vec<&str> = all.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|graph| graph["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["third", "second", "first"]);
    assert_eq!(all.body["count"], 3);
    let limited = get(&app, "/api/v1/graphs?limit=2").await;
    assert_eq!(limited.body["count"], 2);
}

#[tokio::test]
async fn invalid_limits_are_400() {
    let app = app().await;
    for (uri, code) in [
        ("/api/v1/graphs?limit=0", "INVALID_REQUEST"),
        ("/api/v1/graphs?limit=101", "INVALID_REQUEST"),
        ("/api/v1/graphs?limit=ten", "INVALID_QUERY"),
    ] {
        assert_error(&get(&app, uri).await, StatusCode::BAD_REQUEST, code);
    }
}

#[tokio::test]
async fn get_graph_returns_200_or_404() {
    let app = app().await;
    let created = create(&app, "Knowledge").await;
    let id = created["id"].as_str().unwrap();
    let found = get(&app, &format!("/api/v1/graphs/{id}")).await;
    assert_eq!(found.status, StatusCode::OK);
    assert_eq!(found.body, created);

    let missing = get(&app, &format!("/api/v1/graphs/{}", GraphId::new())).await;
    assert_error(&missing, StatusCode::NOT_FOUND, "GRAPH_NOT_FOUND");
    assert_eq!(missing.body["error"]["message"], "Graph was not found");
}

#[tokio::test]
async fn invalid_graph_ids_are_400() {
    let app = app().await;
    for id in [
        "not-a-uuid",
        "00000000-0000-0000-0000-000000000000",
        "9b2f4c1e-3a5d-4e8f-9c7b-1d2e3f4a5b6c", // valid UUIDv4, not a MindGraph id
    ] {
        let reply = get(&app, &format!("/api/v1/graphs/{id}")).await;
        assert_error(&reply, StatusCode::BAD_REQUEST, "INVALID_GRAPH_ID");
        let ontology = get(&app, &format!("/api/v1/graphs/{id}/ontology")).await;
        assert_error(&ontology, StatusCode::BAD_REQUEST, "INVALID_GRAPH_ID");
    }
}

#[tokio::test]
async fn new_graphs_expose_an_empty_ontology_v1() {
    let app = app().await;
    let id = create(&app, "Fraud").await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reply = get(&app, &format!("/api/v1/graphs/{id}/ontology")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        json!({ "graph_id": id, "version": 1, "entity_types": [], "relationship_types": [] })
    );
    let missing = get(&app, &format!("/api/v1/graphs/{}/ontology", GraphId::new())).await;
    assert_error(&missing, StatusCode::NOT_FOUND, "GRAPH_NOT_FOUND");
}

#[tokio::test]
async fn request_ids_are_generated_or_echoed() {
    let app = app().await;
    let generated = get(&app, "/health").await;
    let id = generated.headers["x-request-id"].to_str().unwrap();
    assert!(uuid::Uuid::parse_str(id).is_ok(), "{id}");

    let echoed = send(
        &app,
        Request::get("/health")
            .header("x-request-id", "client-trace-42")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(echoed.headers["x-request-id"], "client-trace-42");

    let replaced = send(
        &app,
        Request::get("/health")
            .header("x-request-id", "has spaces")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_ne!(replaced.headers["x-request-id"], "has spaces");

    let failed = send(
        &app,
        Request::get(format!("/api/v1/graphs/{}", GraphId::new()))
            .header("x-request-id", "trace-for-error")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(failed.body["error"]["request_id"], "trace-for-error");
}

#[tokio::test]
async fn unknown_routes_and_methods_use_the_envelope() {
    let app = app().await;
    assert_error(
        &get(&app, "/api/v1/nope").await,
        StatusCode::NOT_FOUND,
        "ROUTE_NOT_FOUND",
    );
    assert_error(
        &get(&app, "/api/v2/graphs").await,
        StatusCode::NOT_FOUND,
        "ROUTE_NOT_FOUND",
    );
    let delete = send(
        &app,
        Request::delete(format!("/api/v1/graphs/{}", GraphId::new()))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_error(
        &delete,
        StatusCode::METHOD_NOT_ALLOWED,
        "METHOD_NOT_ALLOWED",
    );
}

/// A metadata store whose backend fails after the local workspace exists.
struct FailingStore {
    workspace: Workspace,
    unavailable: bool,
}

impl FailingStore {
    fn error(&self) -> MetadataStoreError {
        if self.unavailable {
            MetadataStoreError::BackendUnavailable {
                message: "connection refused".into(),
                source: None,
            }
        } else {
            MetadataStoreError::CorruptData("relation \"graph_projects\" exploded".into())
        }
    }
}

#[async_trait]
impl MetadataStore for FailingStore {
    async fn ensure_local_workspace(&self) -> Result<Workspace, MetadataStoreError> {
        Ok(self.workspace.clone())
    }
    async fn get_workspace(&self, _: WorkspaceId) -> Result<Option<Workspace>, MetadataStoreError> {
        Err(self.error())
    }
    async fn create_graph(&self, _: &GraphProject, _: &Ontology) -> Result<(), MetadataStoreError> {
        Err(self.error())
    }
    async fn get_graph(&self, _: GraphId) -> Result<Option<GraphProject>, MetadataStoreError> {
        Err(self.error())
    }
    async fn list_graphs(
        &self,
        _: WorkspaceId,
        _: usize,
    ) -> Result<Vec<GraphProject>, MetadataStoreError> {
        Err(self.error())
    }
    async fn create_ontology(&self, _: &Ontology) -> Result<(), MetadataStoreError> {
        Err(self.error())
    }
    async fn get_ontology(
        &self,
        _: GraphId,
        _: OntologyVersion,
    ) -> Result<Option<Ontology>, MetadataStoreError> {
        Err(self.error())
    }
    async fn get_active_ontology(
        &self,
        _: GraphId,
    ) -> Result<Option<Ontology>, MetadataStoreError> {
        Err(self.error())
    }
    async fn set_active_ontology(
        &self,
        _: GraphId,
        _: OntologyVersion,
    ) -> Result<(), MetadataStoreError> {
        Err(self.error())
    }
    async fn ping(&self) -> Result<(), MetadataStoreError> {
        Err(self.error())
    }
}

async fn failing_app(unavailable: bool) -> Router {
    let workspace = MemoryMetadataStore::new()
        .ensure_local_workspace()
        .await
        .unwrap();
    app_with(Arc::new(FailingStore {
        workspace,
        unavailable,
    }))
    .await
}

#[tokio::test]
async fn unavailable_database_is_503() {
    let app = failing_app(true).await;
    assert_error(
        &get(&app, "/ready").await,
        StatusCode::SERVICE_UNAVAILABLE,
        "NOT_READY",
    );
    assert_eq!(
        get(&app, "/health").await.status,
        StatusCode::OK,
        "liveness does not need the database"
    );
    let create = post_json(&app, "/api/v1/graphs", json!({ "name": "x" })).await;
    assert_error(
        &create,
        StatusCode::SERVICE_UNAVAILABLE,
        "SERVICE_UNAVAILABLE",
    );
}

#[tokio::test]
async fn unexpected_failures_are_500_without_internal_details() {
    let app = failing_app(false).await;
    let reply = get(&app, "/api/v1/graphs").await;
    assert_error(&reply, StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR");
    let message = reply.body["error"]["message"].as_str().unwrap();
    assert_eq!(message, "An unexpected error occurred");
    assert!(!reply.body.to_string().contains("graph_projects"));
}
