//! M5 HTTP API tests: AI Graph Builder suggestions through the real router,
//! with in-memory stores and a fake AI (no AI service or provider).

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use mindgraph_api::{AppState, router};
use mindgraph_core::{
    AiError, AiMetadata, AiStatus, AiSuggestion, GraphBuilderAi, GraphBuilderConfig,
    GraphBuilderRequest, GraphBuilderService, GraphProjectService, IngestionConfig, IngestionPorts,
    IngestionService, MemoryMetadataStore, MetadataStore, OntologyService,
};
use mindgraph_graph::MemoryGraphStore;
use mindgraph_ingestion::MemorySourceStore;
use serde_json::{Value, json};
use tower::ServiceExt;

const BOUNDARY: &str = "mindgraph-test-boundary";
const CUSTOMERS: &str = "customer_id,name\nC001,Ann\nC002,Bob\n";

/// Answers every suggestion with a valid Customer suggestion.
struct FakeAi {
    status: AiStatus,
    calls: Mutex<usize>,
}

#[async_trait]
impl GraphBuilderAi for FakeAi {
    async fn status(&self) -> AiStatus {
        self.status.clone()
    }

    async fn suggest(&self, request: &GraphBuilderRequest) -> Result<AiSuggestion, AiError> {
        *self.calls.lock().unwrap() += 1;
        // The same customer mapping for every dataset of the request.
        let mappings: Vec<Value> = request
            .datasets
            .iter()
            .map(|dataset| {
                json!({"dataset": dataset.name, "mapping": {"entities": [{"alias": "customer",
                    "entity_type": "Customer", "mode": "create_or_match",
                    "identity": [{"source_field": "customer_id", "property": "customer_id"}],
                    "properties": [
                        {"source_field": "customer_id", "property": "customer_id"},
                        {"source_field": "name", "property": "name"}]}],
                    "relationships": []}})
            })
            .collect();
        Ok(AiSuggestion {
            suggestion: serde_json::from_value(json!({
                "ontology": {"entity_types": [{"name": "Customer", "display_name": "Customer",
                    "properties": [
                        {"name": "customer_id", "type": "string", "required": true},
                        {"name": "name", "type": "string", "required": false}]}],
                    "relationship_types": []},
                "mappings": mappings,
                "confidence": 0.92,
                "annotations": {"entities": [{"name": "Customer", "confidence": 0.97,
                    "reason": "customer_id identifies each record"}], "relationships": []},
                "warnings": []
            }))
            .unwrap(),
            metadata: AiMetadata {
                prompt_version: "graph_builder_v1".into(),
                provider: "openai_compatible".into(),
                model: "test-model".into(),
            },
        })
    }
}

struct TestApp {
    router: Router,
    ai: Option<Arc<FakeAi>>,
}

/// `ai = None` builds the router without an AI Graph Builder.
async fn app(ai: Option<AiStatus>) -> TestApp {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let graphs = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let workspace_id = graphs.workspace_id();
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata: metadata.clone(),
            graph: Arc::new(MemoryGraphStore::new()),
            sources: Arc::new(MemorySourceStore::new()),
        },
        workspace_id,
        IngestionConfig::default(),
    )
    .unwrap();
    let fake = ai.map(|status| {
        Arc::new(FakeAi {
            status,
            calls: Mutex::new(0),
        })
    });
    let mut state = AppState::new(
        graphs,
        OntologyService::new(metadata, workspace_id),
        ingestion.clone(),
    );
    if let Some(fake) = &fake {
        let ai: Arc<dyn GraphBuilderAi> = fake.clone();
        state = state.with_graph_builder(
            GraphBuilderService::new(ai, &ingestion, GraphBuilderConfig::default()).unwrap(),
        );
    }
    TestApp {
        router: router(state),
        ai: fake,
    }
}

fn ready() -> Option<AiStatus> {
    Some(AiStatus::Ready {
        provider: "openai_compatible".into(),
        model: "test-model".into(),
    })
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

async fn send(app: &TestApp, request: Request<Body>) -> Reply {
    let response = app.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

async fn get(app: &TestApp, uri: &str) -> Reply {
    send(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

async fn post(app: &TestApp, uri: &str, key: Option<&str>) -> Reply {
    let mut request = Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", "client-request-1");
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    send(app, request.body(Body::from("{}")).unwrap()).await
}

async fn create_graph(app: &TestApp) -> String {
    let request = Request::post("/api/v1/graphs")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"name": "Fraud"}"#))
        .unwrap();
    send(app, request).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn upload(app: &TestApp, graph: &str) -> String {
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"customers.csv\"\r\n\r\n{CUSTOMERS}\r\n--{BOUNDARY}--\r\n"
    );
    let request = Request::post(format!("/api/v1/graphs/{graph}/data-sources"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap();
    send(app, request).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn wait(app: &TestApp, job: &str) -> Value {
    for _ in 0..500 {
        let reply = get(app, &format!("/api/v1/jobs/{job}")).await;
        if matches!(reply.body["status"].as_str(), Some("completed" | "failed")) {
            return reply.body;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("job {job} did not finish");
}

async fn analysed(app: &TestApp, graph: &str) -> String {
    let source = upload(app, graph).await;
    let accepted = post(
        app,
        &format!("/api/v1/graphs/{graph}/data-sources/{source}/analyze"),
        None,
    )
    .await;
    let job = wait(app, accepted.body["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed");
    source
}

fn suggestions(graph: &str, source: &str) -> String {
    format!("/api/v1/graphs/{graph}/data-sources/{source}/graph-builder/suggestions")
}

fn assert_error(reply: &Reply, status: StatusCode, code: &str) {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert_eq!(reply.body["error"]["code"], code, "{}", reply.body);
    assert_eq!(reply.headers["x-request-id"], "client-request-1");
    assert_eq!(reply.body["error"]["request_id"], "client-request-1");
}

#[tokio::test]
async fn suggestions_are_accepted_and_returned_through_the_job_api() {
    let app = app(ready()).await;
    let graph = create_graph(&app).await;
    let source = analysed(&app, &graph).await;

    let accepted = post(&app, &suggestions(&graph, &source), Some("suggest-1")).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.body);
    assert_eq!(accepted.body["status"], "queued");
    assert_eq!(accepted.headers["x-request-id"], "client-request-1");
    let job_id = accepted.body["job_id"].as_str().unwrap();
    assert_eq!(
        accepted.headers[header::LOCATION],
        format!("/api/v1/jobs/{job_id}").as_str()
    );

    let job = wait(&app, job_id).await;
    assert_eq!(job["kind"], "ai_suggest");
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(job["ontology_version"], 1, "pinned");
    let result = &job["result"];
    assert_eq!(result["prompt_version"], "graph_builder_v1");
    assert_eq!(result["model"], "test-model");
    assert_eq!(result["ontology_version_base"], 1);
    let suggestion = &result["suggestion"];
    assert_eq!(
        suggestion["ontology"]["entity_types"][0]["name"],
        "Customer"
    );
    assert_eq!(job["data_source_ids"], json!([source]));
    let mapping = &suggestion["mappings"][0];
    assert_eq!(mapping["dataset"], "customers.csv");
    assert_eq!(mapping["data_source_id"], source.as_str());
    assert_eq!(mapping["mapping"]["entities"][0]["mode"], "create_or_match");
    assert_eq!(suggestion["annotations"]["entities"][0]["existing"], false);

    // The reviewed suggestion is applied with the unchanged M4 endpoints.
    let ontology = Request::post(format!("/api/v1/graphs/{graph}/ontology"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(suggestion["ontology"].to_string()))
        .unwrap();
    assert_eq!(send(&app, ontology).await.status, StatusCode::CREATED);
    let ingestion = Request::post(format!(
        "/api/v1/graphs/{graph}/data-sources/{source}/ingestions"
    ))
    .header(header::CONTENT_TYPE, "application/json")
    .header("idempotency-key", "ingest-1")
    .body(Body::from(
        json!({"mapping": mapping["mapping"]}).to_string(),
    ))
    .unwrap();
    let ingestion = send(&app, ingestion).await;
    assert_eq!(ingestion.status, StatusCode::ACCEPTED, "{}", ingestion.body);
    let done = wait(&app, ingestion.body["job_id"].as_str().unwrap()).await;
    assert_eq!(done["result"]["entities_created"], 2, "{done}");
}

#[tokio::test]
async fn repeated_keys_return_the_same_job() {
    let app = app(ready()).await;
    let graph = create_graph(&app).await;
    let source = analysed(&app, &graph).await;
    let first = post(&app, &suggestions(&graph, &source), Some("same")).await;
    let job_id = first.body["job_id"].as_str().unwrap().to_owned();
    wait(&app, &job_id).await;
    let again = post(&app, &suggestions(&graph, &source), Some("same")).await;
    assert_eq!(again.status, StatusCode::ACCEPTED);
    assert_eq!(again.body["job_id"], job_id.as_str());
    assert_eq!(*app.ai.as_ref().unwrap().calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn invalid_requests_are_rejected() {
    let app = app(ready()).await;
    let graph = create_graph(&app).await;
    let other = create_graph(&app).await;
    let pending = upload(&app, &graph).await;
    let source = analysed(&app, &graph).await;

    assert_error(
        &post(&app, &suggestions(&graph, &source), None).await,
        StatusCode::BAD_REQUEST,
        "MISSING_IDEMPOTENCY_KEY",
    );
    assert_error(
        &post(&app, &suggestions(&graph, &pending), Some("k")).await,
        StatusCode::CONFLICT,
        "CONFLICT",
    );
    assert_error(
        &post(&app, &suggestions(&other, &source), Some("k")).await,
        StatusCode::NOT_FOUND,
        "DATA_SOURCE_NOT_FOUND",
    );
    assert_error(
        &post(&app, &suggestions(&graph, "nope"), Some("k")).await,
        StatusCode::BAD_REQUEST,
        "INVALID_DATA_SOURCE_ID",
    );
    assert_eq!(*app.ai.as_ref().unwrap().calls.lock().unwrap(), 0);
}

#[tokio::test]
async fn disabled_or_unavailable_ai_is_503_and_manual_flows_still_work() {
    for (ai, code, status) in [
        (None, "AI_NOT_CONFIGURED", "not_configured"),
        (
            Some(AiStatus::NotConfigured),
            "AI_NOT_CONFIGURED",
            "not_configured",
        ),
        (Some(AiStatus::Unavailable), "AI_UNAVAILABLE", "unavailable"),
    ] {
        let app = app(ai).await;
        let graph = create_graph(&app).await;
        let source = analysed(&app, &graph).await;
        assert_error(
            &post(&app, &suggestions(&graph, &source), Some("k")).await,
            StatusCode::SERVICE_UNAVAILABLE,
            code,
        );
        let reported = get(&app, "/api/v1/graph-builder/status").await;
        assert_eq!(reported.status, StatusCode::OK);
        assert_eq!(reported.body["status"], status);
        assert!(reported.body.get("provider").is_none());
        // Readiness and the M4 manual path do not depend on AI.
        assert_eq!(get(&app, "/ready").await.status, StatusCode::OK);
        let profile = get(
            &app,
            &format!("/api/v1/graphs/{graph}/data-sources/{source}"),
        )
        .await;
        assert_eq!(profile.body["status"], "ready");
    }
}

#[tokio::test]
async fn status_reports_provider_and_model_but_no_urls() {
    let app = app(ready()).await;
    let reply = get(&app, "/api/v1/graph-builder/status").await;
    assert_eq!(
        reply.body,
        json!({"status": "ready", "configured": true,
               "provider": "openai_compatible", "model": "test-model"})
    );
}

async fn post_body(app: &TestApp, uri: &str, key: Option<&str>, body: &Value) -> Reply {
    let mut request = Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", "client-request-1");
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    send(app, request.body(Body::from(body.to_string())).unwrap()).await
}

#[tokio::test]
async fn one_suggestion_can_model_several_data_sources() {
    let app = app(ready()).await;
    let graph = create_graph(&app).await;
    let first = analysed(&app, &graph).await;
    let second = analysed(&app, &graph).await;
    let uri = format!("/api/v1/graphs/{graph}/graph-builder/suggestions");

    let body = json!({"data_source_ids": [second, first]});
    let accepted = post_body(&app, &uri, Some("both"), &body).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.body);
    let job = wait(&app, accepted.body["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(job["data_source_id"], second.as_str());
    assert_eq!(job["data_source_ids"], json!([second, first]));
    let mappings = job["result"]["suggestion"]["mappings"].as_array().unwrap();
    let linked: Vec<(&str, &str)> = mappings
        .iter()
        .map(|m| {
            (
                m["dataset"].as_str().unwrap(),
                m["data_source_id"].as_str().unwrap(),
            )
        })
        .collect();
    // Same filename twice: distinct labels; no dependencies: client order.
    assert_eq!(
        linked,
        [
            ("customers.csv", second.as_str()),
            ("customers.csv (2)", first.as_str())
        ]
    );

    let again = post_body(&app, &uri, Some("both"), &body).await;
    assert_eq!(again.body["job_id"], accepted.body["job_id"]);
    let reordered = json!({"data_source_ids": [first, second]});
    assert_error(
        &post_body(&app, &uri, Some("both"), &reordered).await,
        StatusCode::CONFLICT,
        "CONFLICT",
    );
    for (bad, status, code) in [
        (
            json!({"data_source_ids": []}),
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
        ),
        (
            json!({"data_source_ids": [first, first]}),
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
        ),
        (
            json!({"data_source_ids": ["nope"]}),
            StatusCode::BAD_REQUEST,
            "INVALID_DATA_SOURCE_ID",
        ),
        (
            json!({"sources": [first]}),
            StatusCode::BAD_REQUEST,
            "INVALID_JSON",
        ),
    ] {
        assert_error(
            &post_body(&app, &uri, Some("other"), &bad).await,
            status,
            code,
        );
    }
    assert_error(
        &post_body(&app, &uri, None, &body).await,
        StatusCode::BAD_REQUEST,
        "MISSING_IDEMPOTENCY_KEY",
    );
}
