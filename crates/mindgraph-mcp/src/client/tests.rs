//! The Core REST adapter against a mock HTTP server: request shapes,
//! response decoding, graph checks and every failure mode.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::time::Duration;

use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::matchers::{body_json, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::config::{CORE_URL_VAR, Config, GRAPH_ID_VAR, MAX_RESULT_BYTES_VAR, TIMEOUT_VAR};

const GRAPH: &str = "019a5f6e-8b3c-7d21-9f00-1a2b3c4d5e6f";
const OTHER_GRAPH: &str = "019a5f6e-8b3c-7d21-9f00-ffffffffffff";
const ALICE: &str = "019a5f6e-8b3c-7d21-9f00-000000000001";
const DEVICE: &str = "019a5f6e-8b3c-7d21-9f00-000000000002";
const USES: &str = "019a5f6e-8b3c-7d21-9f00-000000000003";

fn config(server: &MockServer, extra: &[(&'static str, &str)]) -> Config {
    let mut vars = vec![
        (CORE_URL_VAR, server.uri()),
        (GRAPH_ID_VAR, GRAPH.to_owned()),
    ];
    vars.extend(extra.iter().map(|(k, v)| (*k, (*v).to_owned())));
    Config::from_lookup(|name| {
        vars.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
    })
    .unwrap()
}

async fn client(server: &MockServer) -> CoreClient {
    CoreClient::new(&config(server, &[])).unwrap()
}

fn graph_path(suffix: &str) -> String {
    format!("/api/v1/graphs/{GRAPH}{suffix}")
}

fn entity(id: &str, graph: &str, entity_type: &str, key: &str, value: &str) -> Value {
    json!({
        "id": id, "graph_id": graph, "entity_type": entity_type,
        "properties": {key: value}, "property_types": {key: "string"}
    })
}

fn alice(graph: &str) -> Value {
    let mut alice = entity(ALICE, graph, "Customer", "customer_id", "C001");
    alice["properties"]["score"] = json!("9007199254740993");
    alice["property_types"]["score"] = json!("integer");
    alice
}

fn uses() -> Value {
    json!({
        "id": USES, "graph_id": GRAPH, "relationship_type": "USES",
        "source_id": ALICE, "target_id": DEVICE, "properties": {}, "property_types": {}
    })
}

fn envelope(code: &str, message: &str) -> Value {
    json!({"error": {"code": code, "message": message, "request_id": "core-request-1"}})
}

#[tokio::test]
async fn health_and_graph_info_are_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"status": "ok", "service": "mindgraph-core", "version": "0.1.0"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(graph_path("")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": GRAPH, "workspace_id": "019a5f6e-0000-7000-8000-000000000000",
            "name": "Fraud Investigation", "description": "Synthetic fraud graph",
            "status": "active", "created_at": "2026-11-05T10:00:00Z", "updated_at": "2026-11-05T10:05:00Z"
        })))
        .mount(&server)
        .await;
    let client = client(&server).await;
    let health = client.health().await.unwrap();
    assert_eq!(health.service, "mindgraph-core");
    let graph = client.graph().await.unwrap();
    assert_eq!(graph.name, "Fraud Investigation");
    assert_eq!(graph.status, "active");
}

#[tokio::test]
async fn the_ontology_is_read_for_the_configured_graph() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(graph_path("/ontology")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "graph_id": GRAPH, "version": 3,
            "entity_types": [{"id": "x", "name": "Customer", "display_name": "Customer",
                "description": null, "properties": [{"key": "customer_id", "type": "string", "required": true}]}],
            "relationship_types": [{"id": "y", "name": "OWNS", "source_entity_type": "Customer",
                "target_entity_type": "Account", "properties": []}]
        })))
        .mount(&server)
        .await;
    let ontology = client(&server).await.ontology().await.unwrap();
    let schema = crate::dto::GraphSchema::from(ontology);
    assert_eq!(schema.version, 3);
    assert_eq!(schema.entity_types[0].properties[0].name, "customer_id");
    assert!(schema.entity_types[0].properties[0].required);
    assert_eq!(schema.relationship_types[0].target_entity_type, "Account");
}

#[tokio::test]
async fn searches_send_typed_filters_and_keep_large_integers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(graph_path("/entities/search")))
        .and(body_json(json!({
            "entity_type": "Customer", "property_filters": {"customer_id": "C001"}, "limit": 20
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": [alice(GRAPH)], "count": 1, "limit": 20})),
        )
        .mount(&server)
        .await;
    let mut filters = serde_json::Map::new();
    filters.insert("customer_id".into(), json!("C001"));
    let entities = client(&server)
        .await
        .search_entities(Some("Customer"), &filters, 20)
        .await
        .unwrap();
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].id, ALICE);
    assert_eq!(entities[0].properties["score"], json!("9007199254740993"));
    assert_eq!(entities[0].property_types["score"], "integer");
}

#[tokio::test]
async fn entity_neighbors_path_and_subgraph_use_the_explorer_endpoints() {
    let server = MockServer::start().await;
    let device = entity(DEVICE, GRAPH, "Device", "device_id", "DEVICE-921");
    Mock::given(method("GET"))
        .and(path(graph_path(&format!("/entities/{ALICE}"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(alice(GRAPH)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(graph_path(&format!("/entities/{ALICE}/neighbors"))))
        .and(query_param("direction", "outgoing"))
        .and(query_param("relationship_type", "USES"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"entities": [alice(GRAPH), device.clone()], "relationships": [uses()]}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(graph_path("/paths")))
        .and(body_json(json!({
            "source_id": ALICE, "target_id": DEVICE, "direction": "both", "max_depth": 3
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "found": true, "hops": 1, "entities": [alice(GRAPH), device.clone()], "relationships": [uses()]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(graph_path("/subgraph")))
        .and(body_json(json!({"entity_ids": [ALICE, DEVICE]})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"entities": [alice(GRAPH), device], "relationships": [uses()]}),
            ),
        )
        .mount(&server)
        .await;
    let client = client(&server).await;
    let alice_id: Uuid = ALICE.parse().unwrap();
    let device_id: Uuid = DEVICE.parse().unwrap();

    assert_eq!(
        client.entity(alice_id).await.unwrap().entity_type,
        "Customer"
    );
    let neighbors = client
        .neighbors(alice_id, Direction::Outgoing, Some("USES"), 50)
        .await
        .unwrap();
    assert_eq!(neighbors.entities.len(), 2);
    assert_eq!(neighbors.relationships[0].target_id, DEVICE);
    let path = client
        .find_path(alice_id, device_id, Direction::Both, 3)
        .await
        .unwrap();
    assert!(path.found);
    assert_eq!(path.hops, 1);
    let subgraph = client.subgraph(&[alice_id, device_id]).await.unwrap();
    assert_eq!(subgraph.relationships.len(), 1);
}

#[tokio::test]
async fn graphrag_questions_use_the_m7_contract() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(graph_path("/ask")))
        .and(body_json(json!({
            "question": "Who shares this device?",
            "scope": {"mode": "selected_entities", "entity_ids": [DEVICE]}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "answered", "answer": "Alice uses it [E1].", "answer_source": "model",
            "evidence": {"entities": [], "relationships": [], "paths": []},
            "citations": [{"ref": "E1", "kind": "entity", "id": ALICE}],
            "warnings": [], "retrieval": {"plan": {"operation": "neighbors"}},
            "metadata": {"conversation_turns": 0}
        })))
        .mount(&server)
        .await;
    let answer = client(&server)
        .await
        .ask("Who shares this device?", &[DEVICE.parse().unwrap()])
        .await
        .unwrap();
    assert_eq!(answer.status, "answered");
    assert_eq!(answer.citations[0]["ref"], "E1");
}

#[tokio::test]
async fn core_error_envelopes_become_tool_errors() {
    let server = MockServer::start().await;
    for (status, code, suffix) in [
        (404, "ENTITY_NOT_FOUND", format!("/entities/{ALICE}")),
        (400, "INVALID_QUERY", "/ontology".to_owned()),
        (503, "SERVICE_UNAVAILABLE", String::new()),
    ] {
        Mock::given(method("GET"))
            .and(path(graph_path(&suffix)))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(envelope(code, "Core says no")),
            )
            .mount(&server)
            .await;
    }
    let client = client(&server).await;
    let not_found = client.entity(ALICE.parse().unwrap()).await.unwrap_err();
    assert_eq!(not_found.code, "ENTITY_NOT_FOUND");
    assert_eq!(not_found.http_status, Some(404));
    assert_eq!(not_found.message, "Core says no");
    assert_eq!(not_found.request_id.as_deref(), Some("core-request-1"));
    assert!(not_found.hint.is_some());
    assert_eq!(client.ontology().await.unwrap_err().code, "INVALID_QUERY");
    let unavailable = client.graph().await.unwrap_err();
    assert_eq!(unavailable.code, "SERVICE_UNAVAILABLE");
    assert_eq!(unavailable.http_status, Some(503));
}

#[tokio::test]
async fn error_pages_without_an_envelope_are_not_passed_on() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(graph_path("")))
        .respond_with(
            ResponseTemplate::new(502)
                .set_body_raw("<html>Bad gateway: upstream db=secret</html>", "text/html"),
        )
        .mount(&server)
        .await;
    let error = client(&server).await.graph().await.unwrap_err();
    assert_eq!(error.code, codes::CORE_ERROR);
    assert_eq!(error.http_status, Some(502));
    assert!(!error.message.contains("secret"), "{}", error.message);
    assert!(!error.message.contains("html"), "{}", error.message);
}

#[tokio::test]
async fn malformed_and_non_json_answers_are_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(graph_path("")))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{\"id\": ", "application/json"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(graph_path("/ontology")))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("<html>MindGraph</html>", "text/html"),
        )
        .mount(&server)
        .await;
    let client = client(&server).await;
    assert_eq!(
        client.graph().await.unwrap_err().code,
        codes::INVALID_CORE_RESPONSE
    );
    let html = client.ontology().await.unwrap_err();
    assert_eq!(html.code, codes::INVALID_CORE_RESPONSE);
    assert!(html.message.contains("JSON"), "{}", html.message);
}

#[tokio::test]
async fn elements_of_another_graph_are_discarded() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(graph_path(&format!("/entities/{ALICE}"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(alice(OTHER_GRAPH)))
        .mount(&server)
        .await;
    let error = client(&server)
        .await
        .entity(ALICE.parse().unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::INVALID_CORE_RESPONSE);
    assert!(error.message.contains("another graph"), "{}", error.message);
}

#[tokio::test]
async fn oversized_answers_are_refused_not_truncated() {
    let server = MockServer::start().await;
    let big = "x".repeat(40_000);
    Mock::given(method("GET"))
        .and(path(graph_path(&format!("/entities/{ALICE}"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": ALICE, "graph_id": GRAPH, "entity_type": "Customer",
            "properties": {"notes": big}, "property_types": {"notes": "string"}
        })))
        .mount(&server)
        .await;
    // 16 KiB results allow Core bodies of at most 32 KiB.
    let config = config(&server, &[(MAX_RESULT_BYTES_VAR, "16384")]);
    let error = CoreClient::new(&config)
        .unwrap()
        .entity(ALICE.parse().unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::RESULT_TOO_LARGE);
    assert!(error.hint.is_some());
}

#[tokio::test]
async fn slow_answers_time_out() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(graph_path("")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({}))
                .set_delay(Duration::from_secs(3)),
        )
        .mount(&server)
        .await;
    let config = config(&server, &[(TIMEOUT_VAR, "1")]);
    let error = CoreClient::new(&config).unwrap().graph().await.unwrap_err();
    assert_eq!(error.code, codes::CORE_TIMEOUT);
}

#[tokio::test]
async fn redirects_are_never_followed() {
    let server = MockServer::start().await;
    let elsewhere = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(graph_path("")))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/steal", elsewhere.uri())),
        )
        .mount(&server)
        .await;
    let error = client(&server).await.graph().await.unwrap_err();
    assert_eq!(error.code, codes::CORE_REDIRECT);
    assert_eq!(error.http_status, Some(307));
    assert!(
        elsewhere.received_requests().await.unwrap().is_empty(),
        "the redirect target was never contacted"
    );
}

#[tokio::test]
async fn an_unreachable_core_is_reported_without_crashing() {
    // A port nothing listens on: bind, read the port, release it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = Config::from_lookup(|name| match name {
        "MINDGRAPH_MCP_CORE_URL" => Some(format!("http://127.0.0.1:{port}")),
        "MINDGRAPH_MCP_GRAPH_ID" => Some(GRAPH.to_owned()),
        _ => None,
    })
    .unwrap();
    let error = CoreClient::new(&config).unwrap().graph().await.unwrap_err();
    assert_eq!(error.code, codes::CORE_UNAVAILABLE);
    assert!(
        error.message.contains(&port.to_string()),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn every_request_carries_a_fresh_request_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(graph_path(&format!("/entities/{ALICE}"))))
        .and(header_exists(REQUEST_ID_HEADER))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": {"code": "ENTITY_NOT_FOUND", "message": "Entity not found"}
        })))
        .expect(2)
        .mount(&server)
        .await;
    let client = client(&server).await;
    let first = client.entity(ALICE.parse().unwrap()).await.unwrap_err();
    let second = client.entity(ALICE.parse().unwrap()).await.unwrap_err();
    let requests = server.received_requests().await.unwrap();
    let sent: Vec<String> = requests
        .iter()
        .map(|request| {
            request.headers[REQUEST_ID_HEADER]
                .to_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_ne!(sent[0], sent[1]);
    // Without an id in the envelope, the error names the id that was sent.
    assert_eq!(first.request_id.as_deref(), Some(sent[0].as_str()));
    assert_eq!(second.request_id.as_deref(), Some(sent[1].as_str()));
    assert!(Uuid::parse_str(&sent[0]).is_ok_and(|id| id.get_version_num() == 7));
    assert!(
        requests[0].headers["user-agent"]
            .to_str()
            .unwrap()
            .starts_with("mindgraph-mcp/")
    );
}
