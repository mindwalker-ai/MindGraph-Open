//! Offline tests of the HTTP adapter against a local mock of the AI service.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::time::Duration;

use mindgraph_ai_client::{DEFAULT_SUGGEST_TIMEOUT, HttpGraphBuilderAi};
use mindgraph_core::{
    AiError, AiStatus, GraphBuilderAi, GraphBuilderRequest, OntologyContext, OntologyDefinition,
};
use mindgraph_ingestion::DatasetProfile;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request() -> GraphBuilderRequest {
    GraphBuilderRequest {
        correlation_id: "019a0000-0000-7000-8000-000000000001".into(),
        dataset_profile: serde_json::from_value::<DatasetProfile>(json!({
            "record_count": 2,
            "fields": [{"name": "customer_id", "inferred_type": "string", "nullable": false,
                        "non_null_count": 2, "sample_values": ["C001", "C002"]}],
            "sample_rows": [{"customer_id": "C001"}]
        }))
        .unwrap(),
        active_ontology: OntologyContext {
            version: 1,
            definition: OntologyDefinition::default(),
        },
        repair: None,
    }
}

fn suggestion() -> Value {
    json!({
        "ontology": {"entity_types": [{"name": "Customer", "properties": [
            {"name": "customer_id", "type": "string", "required": true}]}],
            "relationship_types": []},
        "mapping": {"entities": [{"alias": "customer", "entity_type": "Customer",
            "identity": [{"source_field": "customer_id", "property": "customer_id"}],
            "properties": [{"source_field": "customer_id", "property": "customer_id"}]}],
            "relationships": []},
        "confidence": 0.9,
        "annotations": {"entities": [{"name": "Customer", "confidence": 0.95, "reason": "id"}],
            "relationships": []},
        "warnings": []
    })
}

fn ok_body() -> Value {
    json!({
        "suggestion": suggestion(),
        "metadata": {"prompt_version": "graph_builder_v1", "provider": "openai_compatible",
                     "model": "test-model"}
    })
}

async fn client(server: &MockServer) -> HttpGraphBuilderAi {
    HttpGraphBuilderAi::new(&server.uri(), DEFAULT_SUGGEST_TIMEOUT).unwrap()
}

fn error(status: u16, code: &str) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .set_body_json(json!({"error": {"code": code, "message": "provider said: secret"}}))
}

#[tokio::test]
async fn successful_suggestions_are_decoded_and_correlated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/graph-builder/suggest"))
        .and(header(
            "x-request-id",
            "019a0000-0000-7000-8000-000000000001",
        ))
        .and(body_partial_json(json!({
            "dataset_profile": {"record_count": 2},
            "active_ontology": {"version": 1, "entity_types": []}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .expect(1)
        .mount(&server)
        .await;
    let result = client(&server).await.suggest(&request()).await.unwrap();
    assert_eq!(result.metadata.prompt_version, "graph_builder_v1");
    assert_eq!(result.metadata.model, "test-model");
    assert_eq!(result.suggestion.ontology.entity_types[0].name, "Customer");
    assert_eq!(result.suggestion.mapping.entities[0].alias, "customer");
}

#[tokio::test]
async fn requests_contain_only_the_profile_and_the_ontology() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    client(&server).await.suggest(&request()).await.unwrap();
    let received = &server.received_requests().await.unwrap()[0];
    let body: Value = serde_json::from_slice(&received.body).unwrap();
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["active_ontology", "dataset_profile"]);
}

#[tokio::test]
async fn readiness_reports_ready_disabled_or_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ready", "configured": true, "provider": "openai_compatible", "model": "m"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(json!({"status": "not_configured", "configured": false})),
        )
        .mount(&server)
        .await;
    let ai = client(&server).await;
    assert_eq!(
        ai.status().await,
        AiStatus::Ready {
            provider: "openai_compatible".into(),
            model: "m".into()
        }
    );
    assert_eq!(ai.status().await, AiStatus::NotConfigured);

    // Nothing listens on a port that was bound and released.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let down =
        HttpGraphBuilderAi::new(&format!("http://127.0.0.1:{port}"), DEFAULT_SUGGEST_TIMEOUT)
            .unwrap();
    assert_eq!(down.status().await, AiStatus::Unavailable);
    assert!(matches!(
        down.suggest(&request()).await,
        Err(AiError::Unavailable(_))
    ));
}

#[tokio::test]
async fn error_responses_map_to_ai_errors_without_their_messages() {
    let cases = [
        (error(503, "AI_NOT_CONFIGURED"), "AI_NOT_CONFIGURED"),
        (error(503, "PROVIDER_UNAVAILABLE"), "AI_UNAVAILABLE"),
        (error(429, "PROVIDER_RATE_LIMITED"), "AI_RATE_LIMITED"),
        (error(504, "PROVIDER_TIMEOUT"), "AI_TIMEOUT"),
        (error(502, "PROVIDER_REJECTED"), "AI_PROVIDER_REJECTED"),
        (error(502, "INVALID_PROVIDER_OUTPUT"), "AI_INVALID_RESPONSE"),
        (error(500, "INTERNAL_ERROR"), "AI_INTERNAL_ERROR"),
    ];
    for (response, code) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(response)
            .mount(&server)
            .await;
        let err = client(&server).await.suggest(&request()).await.unwrap_err();
        assert_eq!(err.code(), code);
        assert!(!err.to_string().contains("secret"), "{err}");
    }
}

#[tokio::test]
async fn invalid_bodies_are_invalid_responses() {
    let mut missing_mapping = ok_body();
    missing_mapping["suggestion"]
        .as_object_mut()
        .unwrap()
        .remove("mapping");
    let mut extra_field = ok_body();
    extra_field["suggestion"]["chain_of_thought"] = json!("...");
    for body in [
        ResponseTemplate::new(200).set_body_string("Here is your result: {}"),
        ResponseTemplate::new(200).set_body_json(missing_mapping),
        ResponseTemplate::new(200).set_body_json(extra_field),
        ResponseTemplate::new(200).set_body_json(json!({"suggestion": null})),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(body)
            .mount(&server)
            .await;
        let err = client(&server).await.suggest(&request()).await.unwrap_err();
        assert!(matches!(err, AiError::InvalidResponse(_)), "{err}");
    }
}

#[tokio::test]
async fn oversized_bodies_are_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(2 * 1024 * 1024)))
        .mount(&server)
        .await;
    let err = client(&server).await.suggest(&request()).await.unwrap_err();
    assert!(matches!(err, AiError::InvalidResponse(_)), "{err}");
}

#[tokio::test]
async fn slow_services_time_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(ok_body())
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;
    let ai = HttpGraphBuilderAi::new(&server.uri(), Duration::from_millis(100)).unwrap();
    assert_eq!(ai.suggest(&request()).await.unwrap_err(), AiError::Timeout);
}
