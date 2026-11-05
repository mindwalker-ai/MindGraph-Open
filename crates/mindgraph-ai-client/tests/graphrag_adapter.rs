//! Offline tests of the GraphRAG HTTP adapter against a local mock of the
//! AI service.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::time::Duration;

use mindgraph_ai_client::{DEFAULT_GRAPHRAG_HTTP_TIMEOUT, HttpGraphRagAi};
use mindgraph_core::{
    AiError, ConversationTurnContext, GraphRagAi, InterpretationLimits, InterpretationRequest,
    KnownEntityContext, OntologyContext, OntologyDefinition, RetrievalCompleteness,
    SynthesisRequest,
};
use mindgraph_graphrag::Operation;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const INTERPRET: &str = "/internal/v1/graphrag/interpret";
const SYNTHESIZE: &str = "/internal/v1/graphrag/synthesize";

fn interpretation_request() -> InterpretationRequest {
    InterpretationRequest {
        correlation_id: "req-42".into(),
        question: "Who else uses this device?".into(),
        ontology: OntologyContext {
            version: 2,
            definition: OntologyDefinition::default(),
        },
        selected_entities: vec![KnownEntityContext {
            reference: "S1".into(),
            entity_type: "Customer".into(),
            properties: json!({"customer_id": "C001"}),
        }],
        conversation: vec![],
        earlier_entities: vec![],
        limits: InterpretationLimits {
            max_traversal_depth: 3,
            max_path_depth: 6,
        },
    }
}

fn synthesis_request() -> SynthesisRequest {
    SynthesisRequest {
        correlation_id: "req-42".into(),
        question: "Who else uses this device?".into(),
        earlier_questions: vec![],
        context: json!({"entities": [{"ref": "E1", "type": "Customer", "properties": {}}]}),
        retrieval: RetrievalCompleteness {
            complete: true,
            truncated: false,
            limitations: vec![],
        },
    }
}

fn metadata(version: &str) -> Value {
    json!({"prompt_version": version, "provider": "openai_compatible", "model": "test-model"})
}

async fn client(server: &MockServer) -> HttpGraphRagAi {
    HttpGraphRagAi::new(&server.uri(), DEFAULT_GRAPHRAG_HTTP_TIMEOUT).unwrap()
}

#[tokio::test]
async fn interpretations_are_decoded_and_correlated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(INTERPRET))
        .and(header("x-request-id", "req-42"))
        .and(body_partial_json(json!({
            "question": "Who else uses this device?",
            "ontology": {"version": 2, "entity_types": [], "relationship_types": []},
            "selected_entities": [{"ref": "S1", "entity_type": "Customer",
                                   "properties": {"customer_id": "C001"}}],
            "limits": {"max_traversal_depth": 3, "max_path_depth": 6}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "intent": {"operation": "shared_neighbor", "source": {"ref": "S1"},
                       "entity_type": "Customer", "shared_entity_type": "Device"},
            "metadata": metadata("graphrag_interpret_v1")
        })))
        .expect(1)
        .mount(&server)
        .await;
    let interpretation = client(&server)
        .await
        .interpret(&interpretation_request())
        .await
        .unwrap();
    assert_eq!(interpretation.intent.operation, Operation::SharedNeighbor);
    assert_eq!(
        interpretation.metadata.prompt_version,
        "graphrag_interpret_v1"
    );
}

#[tokio::test]
async fn follow_ups_carry_the_conversation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(INTERPRET))
        .and(body_partial_json(json!({
            "conversation": [{"question": "Which customers share DEVICE-921?",
                              "entities": ["H1", "H2"]}],
            "earlier_entities": [{"ref": "H1", "entity_type": "Customer"},
                                 {"ref": "H2", "entity_type": "Customer"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "intent": {"operation": "find_path", "source": {"ref": "H1"}, "target": {"ref": "H2"}},
            "metadata": metadata("graphrag_interpret_v1")
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(SYNTHESIZE))
        .and(body_partial_json(json!({
            "earlier_questions": ["Which customers share DEVICE-921?"]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answer": {"answer": "Yes [P1].", "evidence_refs": ["P1"]},
            "metadata": metadata("graphrag_synthesize_v1")
        })))
        .expect(1)
        .mount(&server)
        .await;
    let ai = client(&server).await;
    let customer = |reference: &str| KnownEntityContext {
        reference: reference.into(),
        entity_type: "Customer".into(),
        properties: json!({}),
    };
    let mut interpretation = interpretation_request();
    interpretation.question = "Is there a path between them?".into();
    interpretation.conversation = vec![ConversationTurnContext {
        question: "Which customers share DEVICE-921?".into(),
        entities: vec!["H1".into(), "H2".into()],
    }];
    interpretation.earlier_entities = vec![customer("H1"), customer("H2")];
    let decoded = ai.interpret(&interpretation).await.unwrap();
    assert_eq!(decoded.intent.operation, Operation::FindPath);
    let mut synthesis = synthesis_request();
    synthesis.earlier_questions = vec!["Which customers share DEVICE-921?".into()];
    assert_eq!(
        ai.synthesize(&synthesis)
            .await
            .unwrap()
            .answer
            .evidence_refs,
        ["P1"]
    );
}

#[tokio::test]
async fn first_questions_omit_the_conversation_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(INTERPRET))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "intent": {"operation": "unsupported"},
            "metadata": metadata("graphrag_interpret_v1")
        })))
        .mount(&server)
        .await;
    client(&server)
        .await
        .interpret(&interpretation_request())
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body.get("conversation").is_none() && body.get("earlier_entities").is_none());
}

#[tokio::test]
async fn syntheses_are_decoded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SYNTHESIZE))
        .and(header("x-request-id", "req-42"))
        .and(body_partial_json(json!({
            "context": {"entities": [{"ref": "E1"}]},
            "retrieval": {"complete": true, "truncated": false}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answer": {"answer": "Only [E1].", "evidence_refs": ["E1"],
                       "insufficient_evidence": false, "warnings": []},
            "metadata": metadata("graphrag_synthesize_v1")
        })))
        .mount(&server)
        .await;
    let synthesis = client(&server)
        .await
        .synthesize(&synthesis_request())
        .await
        .unwrap();
    assert_eq!(synthesis.answer.evidence_refs, ["E1"]);
    assert_eq!(synthesis.metadata.model, "test-model");
}

#[tokio::test]
async fn errors_and_unexpected_shapes_map_to_ai_errors() {
    let server = MockServer::start().await;
    let ai = client(&server).await;
    let error = |status: u16, code: &str| {
        ResponseTemplate::new(status)
            .set_body_json(json!({"error": {"code": code, "message": "secret detail"}}))
    };
    let cases = [
        (error(503, "AI_NOT_CONFIGURED"), "AI_NOT_CONFIGURED"),
        (error(429, "PROVIDER_RATE_LIMITED"), "AI_RATE_LIMITED"),
        (error(504, "PROVIDER_TIMEOUT"), "AI_TIMEOUT"),
        (error(502, "INVALID_PROVIDER_OUTPUT"), "AI_INVALID_RESPONSE"),
        // A query string smuggled into the intent is an unknown field.
        (
            ResponseTemplate::new(200).set_body_json(json!({
                "intent": {"operation": "neighbors", "gremlin": "g.V().drop()"},
                "metadata": metadata("graphrag_interpret_v1")
            })),
            "AI_INVALID_RESPONSE",
        ),
        (
            ResponseTemplate::new(200).set_body_json(json!({
                "intent": {"operation": "execute_cypher"},
                "metadata": metadata("graphrag_interpret_v1")
            })),
            "AI_INVALID_RESPONSE",
        ),
    ];
    for (response, code) in cases {
        server.reset().await;
        Mock::given(method("POST"))
            .and(path(INTERPRET))
            .respond_with(response)
            .mount(&server)
            .await;
        let failure = ai.interpret(&interpretation_request()).await.unwrap_err();
        assert_eq!(failure.code(), code);
        assert!(!failure.to_string().contains("secret"), "{failure}");
    }
}

#[tokio::test]
async fn slow_services_time_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SYNTHESIZE))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;
    let ai = HttpGraphRagAi::new(&server.uri(), Duration::from_millis(200)).unwrap();
    assert!(matches!(
        ai.synthesize(&synthesis_request()).await,
        Err(AiError::Timeout)
    ));
}
