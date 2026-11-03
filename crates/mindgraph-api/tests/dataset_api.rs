//! M4 HTTP API tests: ontology creation, uploads, analysis, ingestion, jobs
//! and SSE, against the real router with in-memory stores. No PostgreSQL,
//! HugeGraph, filesystem or AI service needed.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use mindgraph_api::{AppState, router};
use mindgraph_core::{
    GraphProjectService, IngestionConfig, IngestionPorts, IngestionService, MemoryMetadataStore,
    MetadataStore, OntologyService,
};
use mindgraph_domain::{GraphId, TypeName};
use mindgraph_graph::{EntitySearch, GraphStore, MemoryGraphStore};
use mindgraph_ingestion::MemorySourceStore;
use serde_json::{Value, json};
use tower::ServiceExt;

const BOUNDARY: &str = "mindgraph-test-boundary";
const CUSTOMERS: &str = "customer_id,name\nC001,Ann\nC002,Bob\nC003,\n";

struct TestApp {
    router: Router,
    graph: Arc<MemoryGraphStore>,
}

async fn app_with(config: IngestionConfig) -> TestApp {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let graph = Arc::new(MemoryGraphStore::new());
    let graph_projects = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let workspace_id = graph_projects.workspace_id();
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata: metadata.clone(),
            graph: graph.clone(),
            sources: Arc::new(MemorySourceStore::new()),
        },
        workspace_id,
        config,
    )
    .unwrap();
    TestApp {
        router: router(AppState::new(
            graph_projects,
            OntologyService::new(metadata, workspace_id),
            ingestion,
        )),
        graph,
    }
}

async fn app() -> TestApp {
    app_with(IngestionConfig::default()).await
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    text: String,
    body: Value,
}

async fn send(app: &TestApp, request: Request<Body>) -> Reply {
    let response = app.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Reply {
        status,
        headers,
        text,
        body,
    }
}

async fn get(app: &TestApp, uri: &str) -> Reply {
    send(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

async fn post_json(app: &TestApp, uri: &str, body: &Value, key: Option<&str>) -> Reply {
    let mut request = Request::post(uri).header(header::CONTENT_TYPE, "application/json");
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    send(app, request.body(Body::from(body.to_string())).unwrap()).await
}

fn multipart(filename: &str, content: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\nignored\r\n\
         --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
         Content-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

async fn upload(app: &TestApp, graph: &str, filename: &str, content: &[u8]) -> Reply {
    let request = Request::post(format!("/api/v1/graphs/{graph}/data-sources"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart(filename, content)))
        .unwrap();
    send(app, request).await
}

async fn create_graph(app: &TestApp) -> String {
    let reply = post_json(app, "/api/v1/graphs", &json!({"name": "Fraud"}), None).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
    reply.body["id"].as_str().unwrap().to_owned()
}

fn customer_ontology() -> Value {
    json!({
        "entity_types": [{
            "name": "Customer",
            "display_name": "Customer",
            "properties": [
                {"name": "customer_id", "type": "string", "required": true},
                {"name": "name", "type": "string", "required": false}
            ]
        }],
        "relationship_types": []
    })
}

fn customer_mapping() -> Value {
    json!({"mapping": {
        "entities": [{
            "alias": "customer",
            "entity_type": "Customer",
            "mode": "create_or_match",
            "identity": [{"source_field": "customer_id", "property": "customer_id"}],
            "properties": [
                {"source_field": "customer_id", "property": "customer_id"},
                {"source_field": "name", "property": "name"}
            ]
        }],
        "relationships": []
    }})
}

async fn wait_for_job(app: &TestApp, job_id: &str) -> Value {
    for _ in 0..500 {
        let job = get(app, &format!("/api/v1/jobs/{job_id}")).await;
        assert_eq!(job.status, StatusCode::OK, "{}", job.text);
        if matches!(
            job.body["status"].as_str(),
            Some("completed" | "failed" | "cancelled")
        ) {
            return job.body;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("job {job_id} did not finish");
}

async fn analyze(app: &TestApp, graph: &str, id: &str) -> Reply {
    post_json(
        app,
        &format!("/api/v1/graphs/{graph}/data-sources/{id}/analyze"),
        &json!({}),
        None,
    )
    .await
}

/// Uploads and analyses `customers.csv`; returns the data source id.
async fn analysed(app: &TestApp, graph: &str) -> String {
    let uploaded = upload(app, graph, "customers.csv", CUSTOMERS.as_bytes()).await;
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.text);
    let id = uploaded.body["id"].as_str().unwrap().to_owned();
    let accepted = analyze(app, graph, &id).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.text);
    let job = wait_for_job(app, accepted.body["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed", "{job}");
    id
}

fn assert_error(reply: &Reply, status: StatusCode, code: &str) {
    assert_eq!(reply.status, status, "{}", reply.text);
    assert_eq!(reply.body["error"]["code"], code, "{}", reply.text);
    let header_id = reply.headers["x-request-id"].to_str().unwrap();
    assert_eq!(reply.body["error"]["request_id"], header_id);
}

#[tokio::test]
async fn csv_upload_returns_a_pending_data_source_without_paths() {
    let app = app().await;
    let graph = create_graph(&app).await;
    let reply = upload(&app, &graph, "customers.csv", CUSTOMERS.as_bytes()).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
    assert!(reply.headers.contains_key("x-request-id"));
    let body = &reply.body;
    assert_eq!(body["graph_id"], graph);
    assert_eq!(body["kind"], "csv");
    assert_eq!(body["filename"], "customers.csv");
    assert_eq!(body["status"], "pending");
    assert_eq!(body["size_bytes"], CUSTOMERS.len());
    assert_eq!(body["content_sha256"].as_str().unwrap().len(), 64);
    assert!(body["profile"].is_null());
    let id = body["id"].as_str().unwrap();
    assert_eq!(
        reply.headers[header::LOCATION],
        format!("/api/v1/graphs/{graph}/data-sources/{id}").as_str()
    );
    for leak in ["storage_key", "sources/", "memory/", "path"] {
        assert!(!reply.text.contains(leak), "{leak} leaked: {}", reply.text);
    }
}

#[tokio::test]
async fn json_upload_is_accepted() {
    let app = app().await;
    let graph = create_graph(&app).await;
    let reply = upload(&app, &graph, "customers.JSON", br#"[{"customer_id":"C1"}]"#).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
    assert_eq!(reply.body["kind"], "json");
}

#[tokio::test]
async fn invalid_uploads_are_rejected() {
    let app = app_with(IngestionConfig {
        max_upload_bytes: 32,
        ..IngestionConfig::default()
    })
    .await;
    let graph = create_graph(&app).await;
    assert_error(
        &upload(&app, &graph, "notes.txt", b"hello").await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "UNSUPPORTED_FILE_TYPE",
    );
    assert_error(
        &upload(&app, &graph, "big.csv", &[b'x'; 100]).await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "PAYLOAD_TOO_LARGE",
    );
    assert_error(
        &upload(&app, &graph, "empty.csv", b"").await,
        StatusCode::BAD_REQUEST,
        "INVALID_REQUEST",
    );
    assert_error(
        &upload(&app, &GraphId::new().to_string(), "a.csv", b"a\n1\n").await,
        StatusCode::NOT_FOUND,
        "GRAPH_NOT_FOUND",
    );
    // Not multipart at all.
    let reply = post_json(
        &app,
        &format!("/api/v1/graphs/{graph}/data-sources"),
        &json!({}),
        None,
    )
    .await;
    assert_error(
        &reply,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "UNSUPPORTED_MEDIA_TYPE",
    );
    // Multipart without a `file` part.
    let request = Request::post(format!("/api/v1/graphs/{graph}/data-sources"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\n1\r\n--{BOUNDARY}--\r\n"
        )))
        .unwrap();
    assert_error(
        &send(&app, request).await,
        StatusCode::BAD_REQUEST,
        "MISSING_FILE",
    );
}

#[tokio::test]
async fn analysis_is_asynchronous_and_stores_the_profile() {
    let app = app().await;
    let graph = create_graph(&app).await;
    let uploaded = upload(&app, &graph, "customers.csv", CUSTOMERS.as_bytes()).await;
    let id = uploaded.body["id"].as_str().unwrap();
    let accepted = analyze(&app, &graph, id).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.text);
    assert_eq!(accepted.body["status"], "queued");
    let job_id = accepted.body["job_id"].as_str().unwrap();
    assert_eq!(
        accepted.headers[header::LOCATION],
        format!("/api/v1/jobs/{job_id}").as_str()
    );
    let job = wait_for_job(&app, job_id).await;
    assert_eq!(job["kind"], "analyze");
    assert_eq!(job["progress"], 1.0);
    assert!(job["error"].is_null() && job["result"].is_null());

    let source = get(&app, &format!("/api/v1/graphs/{graph}/data-sources/{id}")).await;
    assert_eq!(source.status, StatusCode::OK);
    assert_eq!(source.body["status"], "ready");
    let profile = &source.body["profile"];
    assert_eq!(profile["record_count"], 3);
    assert_eq!(profile["fields"][0]["name"], "customer_id");
    assert_eq!(profile["fields"][0]["inferred_type"], "string");
    assert_eq!(profile["fields"][1]["nullable"], true);
    assert_eq!(profile["sample_rows"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn data_sources_are_isolated_by_graph_and_ids_are_validated() {
    let app = app().await;
    let graph = create_graph(&app).await;
    let other = create_graph(&app).await;
    let uploaded = upload(&app, &graph, "customers.csv", CUSTOMERS.as_bytes()).await;
    let id = uploaded.body["id"].as_str().unwrap();
    assert_error(
        &get(&app, &format!("/api/v1/graphs/{other}/data-sources/{id}")).await,
        StatusCode::NOT_FOUND,
        "DATA_SOURCE_NOT_FOUND",
    );
    assert_error(
        &analyze(&app, &other, id).await,
        StatusCode::NOT_FOUND,
        "DATA_SOURCE_NOT_FOUND",
    );
    assert_error(
        &get(&app, &format!("/api/v1/graphs/{graph}/data-sources/nope")).await,
        StatusCode::BAD_REQUEST,
        "INVALID_DATA_SOURCE_ID",
    );
    assert_error(
        &get(&app, "/api/v1/jobs/nope").await,
        StatusCode::BAD_REQUEST,
        "INVALID_JOB_ID",
    );
    assert_error(
        &get(&app, &format!("/api/v1/jobs/{}", GraphId::new())).await,
        StatusCode::NOT_FOUND,
        "JOB_NOT_FOUND",
    );
}

#[tokio::test]
async fn ontology_creation_returns_the_new_active_version() {
    let app = app().await;
    let graph = create_graph(&app).await;
    let uri = format!("/api/v1/graphs/{graph}/ontology");
    let reply = post_json(&app, &uri, &customer_ontology(), None).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
    assert_eq!(reply.body["version"], 2);
    let customer = &reply.body["entity_types"][0];
    assert_eq!(customer["name"], "Customer");
    assert!(
        customer["id"].as_str().unwrap().len() > 30,
        "ids are server-generated"
    );
    assert_eq!(customer["properties"][0]["key"], "customer_id");
    assert_eq!(customer["properties"][0]["required"], true);
    let active = get(&app, &uri).await;
    assert_eq!(active.body, reply.body);

    let mut invalid = customer_ontology();
    invalid["relationship_types"] = json!([{
        "name": "OWNS", "source_entity_type": "Customer", "target_entity_type": "Account"
    }]);
    assert_error(
        &post_json(&app, &uri, &invalid, None).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_FAILED",
    );
    assert_error(
        &post_json(&app, &uri, &json!({"entity_types": [], "extra": 1}), None).await,
        StatusCode::BAD_REQUEST,
        "INVALID_JSON",
    );
    assert_error(
        &post_json(
            &app,
            &format!("/api/v1/graphs/{}/ontology", GraphId::new()),
            &customer_ontology(),
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        "GRAPH_NOT_FOUND",
    );
}

#[tokio::test]
async fn ingestion_writes_the_graph_and_is_idempotent() {
    let app = app().await;
    let graph = create_graph(&app).await;
    post_json(
        &app,
        &format!("/api/v1/graphs/{graph}/ontology"),
        &customer_ontology(),
        None,
    )
    .await;
    let source = analysed(&app, &graph).await;
    let uri = format!("/api/v1/graphs/{graph}/data-sources/{source}/ingestions");

    assert_error(
        &post_json(&app, &uri, &customer_mapping(), None).await,
        StatusCode::BAD_REQUEST,
        "MISSING_IDEMPOTENCY_KEY",
    );

    let first = post_json(&app, &uri, &customer_mapping(), Some("ingest-1")).await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{}", first.text);
    assert_eq!(first.body["kind"], "ingest");
    assert_eq!(first.body["ontology_version"], 2);
    let job_id = first.body["id"].as_str().unwrap().to_owned();
    assert_eq!(
        first.body["job_id"],
        job_id.as_str(),
        "202 responses carry job_id"
    );
    assert!(!first.text.contains("ingest-1"), "the key is not echoed");

    let again = post_json(&app, &uri, &customer_mapping(), Some("ingest-1")).await;
    assert_eq!(again.status, StatusCode::ACCEPTED);
    assert_eq!(again.body["id"], job_id.as_str(), "same key, same job");

    let job = wait_for_job(&app, &job_id).await;
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(
        job["result"],
        json!({"records_processed": 3, "entities_created": 3, "entities_matched": 0, "relationships_created": 0})
    );
    let search = EntitySearch::new()
        .with_entity_type(TypeName::new("Customer").unwrap())
        .with_limit(100)
        .unwrap();
    let graph_id: GraphId = graph.parse().unwrap();
    assert_eq!(
        app.graph
            .search_entities(graph_id, &search)
            .await
            .unwrap()
            .len(),
        3
    );
    let project = get(&app, &format!("/api/v1/graphs/{graph}")).await;
    assert_eq!(project.body["status"], "active");

    let mut different = customer_mapping();
    different["mapping"]["entities"][0]["properties"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert_error(
        &post_json(&app, &uri, &different, Some("ingest-1")).await,
        StatusCode::CONFLICT,
        "CONFLICT",
    );
}

#[tokio::test]
async fn invalid_ingestion_requests_are_rejected() {
    let app = app().await;
    let graph = create_graph(&app).await;
    post_json(
        &app,
        &format!("/api/v1/graphs/{graph}/ontology"),
        &customer_ontology(),
        None,
    )
    .await;
    let pending = upload(&app, &graph, "customers.csv", CUSTOMERS.as_bytes()).await;
    let pending = pending.body["id"].as_str().unwrap();
    assert_error(
        &post_json(
            &app,
            &format!("/api/v1/graphs/{graph}/data-sources/{pending}/ingestions"),
            &customer_mapping(),
            Some("k"),
        )
        .await,
        StatusCode::CONFLICT,
        "CONFLICT",
    );

    let source = analysed(&app, &graph).await;
    let uri = format!("/api/v1/graphs/{graph}/data-sources/{source}/ingestions");
    let mut invalid = customer_mapping();
    invalid["mapping"]["entities"][0]["entity_type"] = json!("Merchant");
    let reply = post_json(&app, &uri, &invalid, Some("k")).await;
    assert_error(&reply, StatusCode::UNPROCESSABLE_ENTITY, "INVALID_MAPPING");
    assert!(
        reply.body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Merchant")
    );
    assert_error(
        &post_json(
            &app,
            &uri,
            &json!({"mapping": {"entities": "x"}}),
            Some("k"),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_JSON",
    );
    assert_error(
        &post_json(&app, &uri, &customer_mapping(), Some("has space")).await,
        StatusCode::BAD_REQUEST,
        "INVALID_REQUEST",
    );
}

#[tokio::test]
async fn job_events_stream_ends_with_a_terminal_event() {
    let app = app().await;
    let graph = create_graph(&app).await;
    let uploaded = upload(&app, &graph, "customers.csv", CUSTOMERS.as_bytes()).await;
    let id = uploaded.body["id"].as_str().unwrap();
    let accepted = analyze(&app, &graph, id).await;
    let job_id = accepted.body["job_id"].as_str().unwrap();
    wait_for_job(&app, job_id).await;

    let reply = get(&app, &format!("/api/v1/jobs/{job_id}/events")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(
        reply.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    assert!(reply.headers.contains_key("x-request-id"));
    assert!(reply.text.contains("event: completed"), "{}", reply.text);
    let data = reply
        .text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    let event: Value = serde_json::from_str(data).unwrap();
    assert_eq!(event["status"], "completed");
    assert_eq!(event["progress"], 1.0);

    assert_error(
        &get(&app, &format!("/api/v1/jobs/{}/events", GraphId::new())).await,
        StatusCode::NOT_FOUND,
        "JOB_NOT_FOUND",
    );
}

#[tokio::test]
async fn failed_jobs_report_a_code_without_values() {
    let app = app().await;
    let graph = create_graph(&app).await;
    let uploaded = upload(
        &app,
        &graph,
        "bad.csv",
        b"id,name\nsecret-1,Ann\nsecret-2,Bob,extra\n",
    )
    .await;
    let id = uploaded.body["id"].as_str().unwrap();
    let accepted = analyze(&app, &graph, id).await;
    let job = wait_for_job(&app, accepted.body["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "failed");
    assert_eq!(job["error"]["code"], "MALFORMED_DATASET");
    assert!(!job.to_string().contains("secret"), "{job}");
    let source = get(&app, &format!("/api/v1/graphs/{graph}/data-sources/{id}")).await;
    assert_eq!(source.body["status"], "failed");
}
