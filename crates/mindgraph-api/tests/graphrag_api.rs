//! Ask MindGraph HTTP endpoints over the in-memory stores and a scripted AI
//! port. Offline: no PostgreSQL, HugeGraph or model.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use mindgraph_api::{AppState, router};
use mindgraph_core::{
    AiError, AiMetadata, AiStatus, CreateGraphProject, DisabledGraphRagAi, GraphExplorerService,
    GraphProjectService, GraphRagAi, GraphRagConfig, GraphRagService, IngestionConfig,
    IngestionPorts, IngestionService, Interpretation, InterpretationRequest, MemoryMetadataStore,
    MetadataStore, OntologyDefinition, OntologyService, Synthesis, SynthesisRequest,
    SynthesizedAnswer,
};
use mindgraph_domain::{
    Entity, EntityId, GraphId, Properties, PropertyKey, Relationship, RelationshipId, TypeName,
};
use mindgraph_graph::{GraphStore, MemoryGraphStore};
use mindgraph_graphrag::{GraphRagLimits, RetrievalIntent};
use mindgraph_ingestion::MemorySourceStore;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Interprets every question as `intent` and cites every entity and
/// relationship of the context (plus `P1`); records correlation ids.
struct ScriptedAi {
    intent: Result<Value, AiError>,
    correlation_ids: Mutex<Vec<String>>,
}

impl ScriptedAi {
    fn new(intent: Result<Value, AiError>) -> Arc<Self> {
        Arc::new(Self {
            intent,
            correlation_ids: Mutex::default(),
        })
    }
}

fn metadata(version: &str) -> AiMetadata {
    AiMetadata {
        prompt_version: version.into(),
        provider: "fake".into(),
        model: "fake-model".into(),
    }
}

#[async_trait]
impl GraphRagAi for ScriptedAi {
    async fn status(&self) -> AiStatus {
        AiStatus::Ready {
            provider: "fake".into(),
            model: "fake-model".into(),
        }
    }

    async fn interpret(&self, request: &InterpretationRequest) -> Result<Interpretation, AiError> {
        self.correlation_ids
            .lock()
            .unwrap()
            .push(request.correlation_id.clone());
        Ok(Interpretation {
            intent: serde_json::from_value::<RetrievalIntent>(self.intent.clone()?).unwrap(),
            metadata: metadata("graphrag_interpret_v1"),
        })
    }

    async fn synthesize(&self, request: &SynthesisRequest) -> Result<Synthesis, AiError> {
        let refs: Vec<String> = ["entities", "relationships"]
            .iter()
            .flat_map(|kind| {
                request.context[kind]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .map(|item| item["ref"].as_str().unwrap().to_owned())
            .chain(["P1".to_owned()])
            .collect();
        Ok(Synthesis {
            answer: SynthesizedAnswer {
                answer: "C001 and C002 both use DEVICE-921 [E1].".into(),
                evidence_refs: refs,
                insufficient_evidence: false,
                warnings: vec![],
            },
            metadata: metadata("graphrag_synthesize_v1"),
        })
    }
}

struct TestApp {
    router: Router,
    graph: String,
    other_graph: String,
    alice: String,
    budi: String,
    device: String,
    foreign: String,
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

fn entity(graph_id: GraphId, entity_type: &str, values: &[(&str, &str)]) -> Entity {
    let mut properties = Properties::new();
    for (key, value) in values {
        properties
            .insert(PropertyKey::new(*key).unwrap(), *value)
            .unwrap();
    }
    Entity::new(
        EntityId::new(),
        graph_id,
        TypeName::new(entity_type).unwrap(),
        properties,
    )
}

fn link(source: &Entity, target: &Entity) -> Relationship {
    Relationship::new(
        RelationshipId::new(),
        source.graph_id(),
        TypeName::new("USES").unwrap(),
        source.id(),
        target.id(),
        Properties::new(),
    )
}

/// `ai`: `None` builds a router without GraphRAG.
async fn app(ai: Option<Arc<dyn GraphRagAi>>, config: GraphRagConfig) -> TestApp {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let store = Arc::new(MemoryGraphStore::new());
    let graphs = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let workspace_id = graphs.workspace_id();
    let ontologies = OntologyService::new(metadata.clone(), workspace_id);
    let definition: OntologyDefinition = serde_json::from_value(json!({
        "entity_types": [
            {"name": "Customer", "properties": [
                {"name": "customer_id", "type": "string", "required": true},
                {"name": "name", "type": "string"}]},
            {"name": "Device", "properties": [
                {"name": "device_id", "type": "string", "required": true}]}
        ],
        "relationship_types": [
            {"name": "USES", "source_entity_type": "Customer", "target_entity_type": "Device"}
        ]
    }))
    .unwrap();
    let mut ids = Vec::new();
    for name in ["Fraud", "Other"] {
        let graph = graphs
            .create_graph(CreateGraphProject {
                name: name.into(),
                description: None,
            })
            .await
            .unwrap();
        ontologies
            .create_version(graph.id(), &definition)
            .await
            .unwrap();
        ids.push(graph.id());
    }
    let (graph_id, other) = (ids[0], ids[1]);
    let alice = entity(
        graph_id,
        "Customer",
        &[("customer_id", "C001"), ("name", "Alice Tan")],
    );
    let budi = entity(
        graph_id,
        "Customer",
        &[("customer_id", "C002"), ("name", "Budi")],
    );
    let device = entity(graph_id, "Device", &[("device_id", "DEVICE-921")]);
    let lonely = entity(graph_id, "Device", &[("device_id", "DEVICE-447")]);
    let foreign = entity(other, "Customer", &[("customer_id", "C001")]);
    store
        .create_entities(vec![alice.clone(), budi.clone(), device.clone(), lonely])
        .await
        .unwrap();
    store.create_entity(foreign.clone()).await.unwrap();
    store
        .create_relationships(vec![link(&alice, &device), link(&budi, &device)])
        .await
        .unwrap();
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata: metadata.clone(),
            graph: store.clone(),
            sources: Arc::new(MemorySourceStore::new()),
        },
        workspace_id,
        IngestionConfig::default(),
    )
    .unwrap();
    let explorer = GraphExplorerService::new(metadata.clone(), store, workspace_id);
    let mut state = AppState::new(
        graphs,
        OntologyService::new(metadata, workspace_id),
        ingestion,
    )
    .with_graph_explorer(explorer.clone());
    if let Some(ai) = ai {
        state = state.with_graphrag(GraphRagService::new(explorer, ai, config));
    }
    TestApp {
        router: router(state),
        graph: graph_id.to_string(),
        other_graph: other.to_string(),
        alice: alice.id().to_string(),
        budi: budi.id().to_string(),
        device: device.id().to_string(),
        foreign: foreign.id().to_string(),
    }
}

async fn ask(app: &TestApp, graph: &str, body: &Value) -> Reply {
    let request = Request::post(format!("/api/v1/graphs/{graph}/ask"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", "client-request-1")
        .body(Body::from(body.to_string()))
        .unwrap();
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

fn assert_error(reply: &Reply, status: StatusCode, code: &str) {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert_eq!(reply.body["error"]["code"], code, "{}", reply.body);
    assert_eq!(reply.headers["x-request-id"], "client-request-1");
    assert_eq!(reply.body["error"]["request_id"], "client-request-1");
    assert!(reply.body["error"]["message"].as_str().is_some());
}

fn shared_device() -> Result<Value, AiError> {
    Ok(
        json!({"operation": "shared_neighbor", "source": {"ref": "S1"},
              "entity_type": "Customer", "shared_entity_type": "Device"}),
    )
}

fn selected(id: &str) -> Value {
    json!({"mode": "selected_entities", "entity_ids": [id]})
}

#[tokio::test]
async fn a_question_returns_the_answer_and_its_evidence() {
    let ai = ScriptedAi::new(shared_device());
    let app = app(Some(ai.clone()), GraphRagConfig::default()).await;
    let reply = ask(
        &app,
        &app.graph,
        &json!({"question": "Who else uses the same device?", "scope": selected(&app.alice)}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.headers["x-request-id"], "client-request-1");
    let body = &reply.body;
    assert_eq!(body["status"], "answered");
    assert_eq!(body["answer_source"], "model");
    assert_eq!(body["answer"], "C001 and C002 both use DEVICE-921 [E1].");
    let entities = body["evidence"]["entities"].as_array().unwrap();
    let mut ids: Vec<&str> = entities.iter().map(|e| e["id"].as_str().unwrap()).collect();
    ids.sort_unstable();
    let mut expected = vec![app.alice.as_str(), app.budi.as_str(), app.device.as_str()];
    expected.sort_unstable();
    assert_eq!(ids, expected);
    assert_eq!(entities[0]["ref"], "E1");
    assert!(entities[0]["properties"].is_object() && entities[0]["property_types"].is_object());
    let relationships = body["evidence"]["relationships"].as_array().unwrap();
    assert_eq!(relationships.len(), 2);
    assert_eq!(relationships[0]["ref"], "R1");
    let path = &body["evidence"]["paths"][0];
    assert_eq!(path["ref"], "P1");
    assert_eq!(path["entity_ids"].as_array().unwrap().len(), 3);
    let citations = body["citations"].as_array().unwrap();
    assert_eq!(citations.len(), 6, "3 entities, 2 relationships, 1 path");
    assert_eq!(citations[0]["kind"], "entity");
    assert!(citations[0]["id"].is_string());
    assert_eq!(citations[5], json!({"ref": "P1", "kind": "path"}));
    assert_eq!(body["retrieval"]["scope"], "selected_entities");
    assert_eq!(body["retrieval"]["plan"]["operation"], "shared_neighbor");
    assert_eq!(body["retrieval"]["truncated"], false);
    assert_eq!(
        body["metadata"]["interpret_prompt_version"],
        "graphrag_interpret_v1"
    );
    assert_eq!(
        body["metadata"]["synthesize_prompt_version"],
        "graphrag_synthesize_v1"
    );
    // The request id reached the AI port.
    assert_eq!(ai.correlation_ids.lock().unwrap()[0], "client-request-1");
}

#[tokio::test]
async fn follow_ups_carry_earlier_questions_and_their_entities() {
    let ai = ScriptedAi::new(Ok(json!({
        "operation": "find_path", "source": {"ref": "H1"}, "target": {"ref": "H2"}
    })));
    let app = app(Some(ai), GraphRagConfig::default()).await;
    let reply = ask(
        &app,
        &app.graph,
        &json!({
            "question": "How are they connected?",
            "history": [
                {"question": "Which customers share DEVICE-921?",
                 "entity_ids": [app.alice, app.budi, app.foreign]}
            ]
        }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let body = &reply.body;
    assert_eq!(body["metadata"]["conversation_turns"], 1);
    assert_eq!(
        body["retrieval"]["plan"]["source"],
        "entity H1 from an earlier question"
    );
    let path = body["evidence"]["paths"][0]["entity_ids"]
        .as_array()
        .unwrap();
    assert_eq!(path.len(), 3, "Alice, the shared device, Budi");
    // The entity of the other graph was ignored, never resolved.
    assert!(
        body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("from earlier questions")),
        "{body}"
    );
    assert!(!body.to_string().contains(&app.foreign));

    for history in [
        json!([{"question": "a"}, {"question": "b"}, {"question": "c"}, {"question": "d"}]),
        json!([{"question": "a", "entity_ids": ["nope"]}]),
        json!([{"question": "  "}]),
    ] {
        let reply = ask(
            &app,
            &app.graph,
            &json!({"question": "And then?", "history": history}),
        )
        .await;
        assert_error(&reply, StatusCode::BAD_REQUEST, "INVALID_HISTORY");
    }
}

#[tokio::test]
async fn invalid_questions_scopes_and_graphs_are_rejected() {
    let app = app(
        Some(ScriptedAi::new(shared_device())),
        GraphRagConfig::default(),
    )
    .await;
    let graph = app.graph.clone();
    let cases = [
        (
            graph.clone(),
            json!({"question": "  "}),
            StatusCode::BAD_REQUEST,
            "INVALID_QUESTION",
        ),
        (
            graph.clone(),
            json!({}),
            StatusCode::BAD_REQUEST,
            "INVALID_QUESTION",
        ),
        (
            graph.clone(),
            json!({"question": "x".repeat(2_001)}),
            StatusCode::BAD_REQUEST,
            "INVALID_QUESTION",
        ),
        (
            graph.clone(),
            json!({"question": "Who?", "scope": {"mode": "everything"}}),
            StatusCode::BAD_REQUEST,
            "INVALID_SCOPE",
        ),
        (
            graph.clone(),
            json!({"question": "Who?", "scope": selected("not-an-id")}),
            StatusCode::BAD_REQUEST,
            "INVALID_SCOPE",
        ),
        // No query language: unknown fields are rejected.
        (
            graph.clone(),
            json!({"question": "Who?", "gremlin": "g.V().drop()"}),
            StatusCode::BAD_REQUEST,
            "INVALID_JSON",
        ),
        (
            "not-a-uuid".into(),
            json!({"question": "Who?"}),
            StatusCode::BAD_REQUEST,
            "INVALID_GRAPH_ID",
        ),
        (
            GraphId::new().to_string(),
            json!({"question": "Who?"}),
            StatusCode::NOT_FOUND,
            "GRAPH_NOT_FOUND",
        ),
        (
            graph.clone(),
            json!({"question": "Who?", "scope": selected(&EntityId::new().to_string())}),
            StatusCode::NOT_FOUND,
            "ENTITY_NOT_FOUND",
        ),
        // An entity of another graph is missing in this one.
        (
            graph.clone(),
            json!({"question": "Who?", "scope": selected(&app.foreign)}),
            StatusCode::NOT_FOUND,
            "ENTITY_NOT_FOUND",
        ),
        (
            app.other_graph.clone(),
            json!({"question": "Who?", "scope": selected(&app.alice)}),
            StatusCode::NOT_FOUND,
            "ENTITY_NOT_FOUND",
        ),
    ];
    for (graph, body, status, code) in cases {
        assert_error(&ask(&app, &graph, &body).await, status, code);
    }
}

#[tokio::test]
async fn questions_need_a_configured_model() {
    for ai in [
        Some(Arc::new(DisabledGraphRagAi) as Arc<dyn GraphRagAi>),
        None,
    ] {
        let app = app(ai, GraphRagConfig::default()).await;
        let reply = ask(
            &app,
            &app.graph,
            &json!({"question": "Who shares a device?"}),
        )
        .await;
        assert_error(&reply, StatusCode::SERVICE_UNAVAILABLE, "AI_NOT_CONFIGURED");
        let request = Request::get("/api/v1/graphrag/status")
            .body(Body::empty())
            .unwrap();
        let response = app.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["status"], "not_configured");
    }
}

#[tokio::test]
async fn missing_evidence_is_a_successful_insufficient_answer() {
    let ai = ScriptedAi::new(Ok(json!({"operation": "entity_lookup",
        "source": {"entity_type": "Customer", "match": {"customer_id": "C999"}}})));
    let app = app(Some(ai), GraphRagConfig::default()).await;
    let reply = ask(&app, &app.graph, &json!({"question": "Who is C999?"})).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["status"], "insufficient_evidence");
    assert_eq!(reply.body["answer_source"], "no_evidence");
    assert_eq!(reply.body["evidence"]["entities"], json!([]));
    assert_eq!(reply.body["retrieval"]["scope"], "bounded_graph");
}

#[tokio::test]
async fn bounded_discovery_that_hits_a_limit_is_partial() {
    let ai = ScriptedAi::new(Ok(json!({"operation": "shared_neighbor",
        "entity_type": "Customer", "shared_entity_type": "Device"})));
    let config = GraphRagConfig {
        limits: GraphRagLimits {
            max_candidates: 1,
            ..GraphRagLimits::default()
        },
        ..GraphRagConfig::default()
    };
    let app = app(Some(ai), config).await;
    let reply = ask(
        &app,
        &app.graph,
        &json!({"question": "Find customers sharing a device", "scope": {"mode": "bounded_graph"}}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["status"], "partial");
    assert_eq!(reply.body["retrieval"]["truncated"], true);
    assert_eq!(reply.body["retrieval"]["candidates_inspected"], 1);
    assert!(
        reply.body["retrieval"]["limitations"][0]
            .as_str()
            .unwrap()
            .contains("Bounded discovery")
    );
}

#[tokio::test]
async fn ai_failures_and_invalid_plans_use_the_error_envelope() {
    let cases = [
        (
            Err(AiError::Timeout),
            StatusCode::GATEWAY_TIMEOUT,
            "AI_TIMEOUT",
        ),
        (
            Err(AiError::Unavailable("down".into())),
            StatusCode::SERVICE_UNAVAILABLE,
            "AI_UNAVAILABLE",
        ),
        (
            Ok(json!({"operation": "neighbors", "source": {"entity_type": "Merchant"}})),
            StatusCode::UNPROCESSABLE_ENTITY,
            "INVALID_RETRIEVAL_PLAN",
        ),
        (
            Ok(json!({"operation": "neighbors", "source": {"ref": "S1"}, "max_depth": 6})),
            StatusCode::UNPROCESSABLE_ENTITY,
            "RETRIEVAL_LIMIT_EXCEEDED",
        ),
    ];
    for (intent, status, code) in cases {
        let app = app(Some(ScriptedAi::new(intent)), GraphRagConfig::default()).await;
        let reply = ask(
            &app,
            &app.graph,
            &json!({"question": "Who?", "scope": selected(&app.alice)}),
        )
        .await;
        assert_error(&reply, status, code);
        assert!(
            !reply.body.to_string().contains("down"),
            "no provider details"
        );
    }
}

/// Never answers in time.
struct SlowAi;

#[async_trait]
impl GraphRagAi for SlowAi {
    async fn status(&self) -> AiStatus {
        AiStatus::Unavailable
    }

    async fn interpret(&self, _: &InterpretationRequest) -> Result<Interpretation, AiError> {
        tokio::time::sleep(Duration::from_secs(10)).await;
        Err(AiError::Timeout)
    }

    async fn synthesize(&self, _: &SynthesisRequest) -> Result<Synthesis, AiError> {
        Err(AiError::Timeout)
    }
}

#[tokio::test]
async fn the_question_deadline_is_a_timeout() {
    let config = GraphRagConfig {
        timeout: Duration::from_secs(1),
        ..GraphRagConfig::default()
    };
    let app = app(Some(Arc::new(SlowAi)), config).await;
    let reply = ask(&app, &app.graph, &json!({"question": "Who?"})).await;
    assert_error(&reply, StatusCode::GATEWAY_TIMEOUT, "AI_TIMEOUT");
}
