//! `GraphRagService` over the in-memory stores with a scripted AI port:
//! validation, scoping, retrieval, citations, statuses, AI failures, the
//! deadline and the concurrency limit. Offline, no model.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mindgraph_core::{
    AiError, AiMetadata, AiStatus, AnswerSource, AnswerStatus, ApplicationError, AskQuestion,
    AskScope, ConversationTurn, CreateGraphProject, DisabledGraphRagAi, GraphExplorerService,
    GraphProjectService, GraphRagAi, GraphRagConfig, GraphRagError, GraphRagService,
    Interpretation, InterpretationRequest, MemoryMetadataStore, MetadataStore, OntologyDefinition,
    OntologyService, Synthesis, SynthesisRequest, SynthesizedAnswer,
};
use mindgraph_domain::{
    Entity, EntityId, GraphId, Properties, PropertyKey, Relationship, RelationshipId, TypeName,
};
use mindgraph_graph::{GraphStore, MemoryGraphStore};
use mindgraph_graphrag::{CitedObject, GraphRagLimits, RetrievalIntent};
use serde_json::{Value, json};

type Synth = Box<dyn Fn(&SynthesisRequest) -> Result<SynthesizedAnswer, AiError> + Send + Sync>;

/// A scripted AI: a fixed interpretation (or error), a synthesis function,
/// an optional delay, and a record of every request.
struct FakeAi {
    intent: Result<Value, AiError>,
    synth: Synth,
    delay: Duration,
    interpretations: Mutex<Vec<InterpretationRequest>>,
    syntheses: Mutex<Vec<SynthesisRequest>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

fn metadata(prompt_version: &str) -> AiMetadata {
    AiMetadata {
        prompt_version: prompt_version.into(),
        provider: "fake".into(),
        model: "fake-model".into(),
    }
}

/// Cites every entity and relationship of the context.
fn cite_everything(request: &SynthesisRequest) -> Result<SynthesizedAnswer, AiError> {
    let refs: Vec<String> = ["entities", "relationships"]
        .iter()
        .flat_map(|kind| {
            request.context[kind]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .map(|item| item["ref"].as_str().unwrap().to_owned())
        .collect();
    Ok(SynthesizedAnswer {
        answer: format!("The graph shows a shared device [{}].", refs.join(", ")),
        evidence_refs: refs,
        insufficient_evidence: false,
        warnings: vec![],
    })
}

impl FakeAi {
    fn new(intent: Value) -> Self {
        Self::scripted(Ok(intent), Box::new(cite_everything))
    }

    fn scripted(intent: Result<Value, AiError>, synth: Synth) -> Self {
        Self {
            intent,
            synth,
            delay: Duration::ZERO,
            interpretations: Mutex::default(),
            syntheses: Mutex::default(),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
        }
    }

    fn slow(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

#[async_trait]
impl GraphRagAi for FakeAi {
    async fn status(&self) -> AiStatus {
        AiStatus::Ready {
            provider: "fake".into(),
            model: "fake-model".into(),
        }
    }

    async fn interpret(&self, request: &InterpretationRequest) -> Result<Interpretation, AiError> {
        self.interpretations.lock().unwrap().push(request.clone());
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        let intent = self.intent.clone()?;
        Ok(Interpretation {
            intent: serde_json::from_value::<RetrievalIntent>(intent).unwrap(),
            metadata: metadata("graphrag_interpret_v1"),
        })
    }

    async fn synthesize(&self, request: &SynthesisRequest) -> Result<Synthesis, AiError> {
        self.syntheses.lock().unwrap().push(request.clone());
        Ok(Synthesis {
            answer: (self.synth)(request)?,
            metadata: metadata("graphrag_synthesize_v1"),
        })
    }
}

struct Fixture {
    explorer: GraphExplorerService,
    graph_id: GraphId,
    alice: EntityId,
    budi: EntityId,
    device: EntityId,
    foreign: EntityId,
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

fn link(kind: &str, source: &Entity, target: &Entity) -> Relationship {
    Relationship::new(
        RelationshipId::new(),
        source.graph_id(),
        TypeName::new(kind).unwrap(),
        source.id(),
        target.id(),
        Properties::new(),
    )
}

fn ontology() -> OntologyDefinition {
    serde_json::from_value(json!({
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
    .unwrap()
}

/// Alice (C001) and Budi (C002) use DEVICE-921; Citra (C003) uses
/// DEVICE-447 alone. Another graph holds a look-alike Alice.
async fn fixture() -> Fixture {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let store = Arc::new(MemoryGraphStore::new());
    let graphs = GraphProjectService::bootstrap(Arc::clone(&metadata))
        .await
        .unwrap();
    let ontologies = OntologyService::new(Arc::clone(&metadata), graphs.workspace_id());
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
            .create_version(graph.id(), &ontology())
            .await
            .unwrap();
        ids.push(graph.id());
    }
    let graph_id = ids[0];
    let alice = entity(
        graph_id,
        "Customer",
        &[("customer_id", "C001"), ("name", "Alice Tan")],
    );
    let budi = entity(
        graph_id,
        "Customer",
        &[("customer_id", "C002"), ("name", "Budi Santoso")],
    );
    let citra = entity(
        graph_id,
        "Customer",
        &[("customer_id", "C003"), ("name", "Citra Lestari")],
    );
    let device = entity(graph_id, "Device", &[("device_id", "DEVICE-921")]);
    let lonely = entity(graph_id, "Device", &[("device_id", "DEVICE-447")]);
    let foreign = entity(
        ids[1],
        "Customer",
        &[("customer_id", "C001"), ("name", "Alice Tan")],
    );
    store
        .create_entities(vec![
            alice.clone(),
            budi.clone(),
            citra.clone(),
            device.clone(),
            lonely.clone(),
        ])
        .await
        .unwrap();
    store.create_entity(foreign.clone()).await.unwrap();
    store
        .create_relationships(vec![
            link("USES", &alice, &device),
            link("USES", &budi, &device),
            link("USES", &citra, &lonely),
        ])
        .await
        .unwrap();
    Fixture {
        explorer: GraphExplorerService::new(metadata, store, graphs.workspace_id()),
        graph_id,
        alice: alice.id(),
        budi: budi.id(),
        device: device.id(),
        foreign: foreign.id(),
    }
}

impl Fixture {
    fn service(&self, ai: Arc<dyn GraphRagAi>, config: GraphRagConfig) -> GraphRagService {
        GraphRagService::new(self.explorer.clone(), ai, config)
    }
}

fn question(text: &str, scope: AskScope) -> AskQuestion {
    AskQuestion {
        question: text.into(),
        scope,
        history: Vec::new(),
        correlation_id: "req-1".into(),
    }
}

fn turn(text: &str, entity_ids: &[EntityId]) -> ConversationTurn {
    ConversationTurn {
        question: text.into(),
        entity_ids: entity_ids.to_vec(),
    }
}

fn shared_device_intent() -> Value {
    json!({"operation": "shared_neighbor", "source": {"ref": "S1"}, "entity_type": "Customer",
           "shared_entity_type": "Device", "relationship_type": "USES"})
}

#[tokio::test]
async fn a_scoped_question_is_answered_from_retrieved_evidence() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::new(shared_device_intent()));
    let service = f.service(ai.clone(), GraphRagConfig::default());
    let answer = service
        .ask(
            f.graph_id,
            question(
                "Who else uses the same device?",
                AskScope::SelectedEntities(vec![f.alice]),
            ),
        )
        .await
        .unwrap();
    assert_eq!(answer.status, AnswerStatus::Answered);
    assert_eq!(answer.source, AnswerSource::Model);
    let mut found: Vec<EntityId> = answer.evidence.entities().iter().map(Entity::id).collect();
    found.sort();
    let mut expected = vec![f.alice, f.budi, f.device];
    expected.sort();
    assert_eq!(found, expected);
    assert_eq!(
        answer.citations.len(),
        5,
        "three entities and two relationships"
    );
    assert!(
        answer
            .citations
            .iter()
            .all(|citation| match citation.object {
                CitedObject::Entity(id) => answer.evidence.entities().iter().any(|e| e.id() == id),
                CitedObject::Relationship(id) =>
                    answer.evidence.relationships().iter().any(|r| r.id() == id),
                CitedObject::Path(index) => index < answer.evidence.paths().len(),
            })
    );
    assert_eq!(answer.retrieval.scope, "selected_entities");
    assert!(!answer.retrieval.truncated);
    assert!(answer.retrieval.context_bytes > 0);
    assert_eq!(
        answer.metadata.synthesize_prompt_version.as_deref(),
        Some("graphrag_synthesize_v1")
    );

    // The model saw references, types and properties: never MindGraph ids.
    let interpretation = ai.interpretations.lock().unwrap()[0].clone();
    assert_eq!(interpretation.selected_entities[0].reference, "S1");
    assert_eq!(interpretation.correlation_id, "req-1");
    let context = serde_json::to_string(&ai.syntheses.lock().unwrap()[0].context).unwrap();
    for id in [f.alice, f.budi, f.device] {
        assert!(!context.contains(&id.to_string()));
    }
    assert!(!context.contains(&f.graph_id.to_string()));
}

#[tokio::test]
async fn a_follow_up_refers_to_entities_of_earlier_questions() {
    let f = fixture().await;
    // "Is there a path between them?" after "Which customers share the device?"
    let ai = Arc::new(FakeAi::new(json!({
        "operation": "find_path", "source": {"ref": "H1"}, "target": {"ref": "H2"}
    })));
    let service = f.service(ai.clone(), GraphRagConfig::default());
    let mut follow_up = question("Is there a path between them?", AskScope::BoundedGraph);
    follow_up.history = vec![
        turn(
            "Which customers share the device?",
            &[f.alice, f.device, f.budi],
        ),
        // An entity of another graph (or one deleted since) is ignored.
        turn("And this one?", &[f.foreign, f.alice]),
    ];
    let answer = service.ask(f.graph_id, follow_up).await.unwrap();
    assert_eq!(answer.status, AnswerStatus::Answered);
    assert_eq!(answer.metadata.conversation_turns, 2);
    assert_eq!(
        answer.retrieval.plan.source.as_deref(),
        Some("entity H1 from an earlier question")
    );
    let path = &answer.evidence.paths()[0];
    assert_eq!(path.entity_ids().first(), Some(&f.alice));
    assert_eq!(path.entity_ids().last(), Some(&f.device));
    assert!(
        answer
            .warnings
            .iter()
            .any(|w| w.contains("1 entity from earlier questions")),
        "{:?}",
        answer.warnings
    );

    // The model saw the earlier questions and references, never ids.
    let interpretation = ai.interpretations.lock().unwrap()[0].clone();
    assert_eq!(interpretation.conversation.len(), 2);
    assert_eq!(interpretation.conversation[0].entities, ["H1", "H2", "H3"]);
    assert_eq!(interpretation.conversation[1].entities, ["H1"]);
    let refs: Vec<&str> = interpretation
        .earlier_entities
        .iter()
        .map(|e| e.reference.as_str())
        .collect();
    assert_eq!(refs, ["H1", "H2", "H3"]);
    assert_eq!(interpretation.earlier_entities[1].entity_type, "Device");
    let sent = format!("{interpretation:?}");
    assert!(!sent.contains(&f.alice.to_string()) && !sent.contains(&f.foreign.to_string()));
    let synthesis = ai.syntheses.lock().unwrap()[0].clone();
    assert_eq!(
        synthesis.earlier_questions,
        ["Which customers share the device?", "And this one?"]
    );
}

#[tokio::test]
async fn selected_entities_keep_their_reference_in_a_follow_up() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::new(shared_device_intent()));
    let service = f.service(ai.clone(), GraphRagConfig::default());
    let mut follow_up = question("And who else?", AskScope::SelectedEntities(vec![f.alice]));
    follow_up.history = vec![turn("Which device does Alice use?", &[f.alice, f.device])];
    service.ask(f.graph_id, follow_up).await.unwrap();
    let interpretation = ai.interpretations.lock().unwrap()[0].clone();
    assert_eq!(interpretation.conversation[0].entities, ["S1", "H1"]);
    assert_eq!(interpretation.earlier_entities.len(), 1);
}

#[tokio::test]
async fn malformed_histories_are_rejected_before_the_model() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::new(shared_device_intent()));
    let service = f.service(ai.clone(), GraphRagConfig::default());
    let too_many_ids: Vec<EntityId> = (0..11).map(|_| EntityId::new()).collect();
    for history in [
        vec![
            turn("a", &[]),
            turn("b", &[]),
            turn("c", &[]),
            turn("d", &[]),
        ],
        vec![turn("   ", &[])],
        vec![turn(&"x".repeat(2_001), &[])],
        vec![turn("a", &too_many_ids)],
    ] {
        let mut follow_up = question("And then?", AskScope::BoundedGraph);
        follow_up.history = history;
        assert!(matches!(
            service.ask(f.graph_id, follow_up).await,
            Err(ApplicationError::GraphRag(GraphRagError::InvalidHistory(_)))
        ));
    }
    assert!(ai.interpretations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn graphs_and_selected_entities_are_checked_first() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::new(shared_device_intent()));
    let service = f.service(ai.clone(), GraphRagConfig::default());
    assert!(matches!(
        service
            .ask(GraphId::new(), question("Who?", AskScope::BoundedGraph))
            .await,
        Err(ApplicationError::GraphNotFound(_))
    ));
    // An entity of another graph is missing here.
    assert!(matches!(
        service
            .ask(f.graph_id, question("Who?", AskScope::SelectedEntities(vec![f.foreign])))
            .await,
        Err(ApplicationError::EntityNotFound(id)) if id == f.foreign
    ));
    for (text, scope) in [
        ("   ", AskScope::BoundedGraph),
        (&*"x".repeat(2_001), AskScope::BoundedGraph),
    ] {
        assert!(matches!(
            service.ask(f.graph_id, question(text, scope)).await,
            Err(ApplicationError::GraphRag(GraphRagError::InvalidQuestion(
                _
            )))
        ));
    }
    let six: Vec<EntityId> = (0..6).map(|_| EntityId::new()).collect();
    assert!(matches!(
        service
            .ask(
                f.graph_id,
                question("Who?", AskScope::SelectedEntities(six))
            )
            .await,
        Err(ApplicationError::GraphRag(GraphRagError::InvalidScope(_)))
    ));
    assert!(
        ai.interpretations.lock().unwrap().is_empty(),
        "no model call for invalid input"
    );
}

#[tokio::test]
async fn missing_evidence_is_insufficient_without_a_model_answer() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::new(json!({"operation": "entity_lookup",
        "source": {"entity_type": "Customer", "match": {"customer_id": "C999"}}})));
    let service = f.service(ai.clone(), GraphRagConfig::default());
    let answer = service
        .ask(f.graph_id, question("Who is C999?", AskScope::BoundedGraph))
        .await
        .unwrap();
    assert_eq!(answer.status, AnswerStatus::InsufficientEvidence);
    assert_eq!(answer.source, AnswerSource::NoEvidence);
    assert!(answer.evidence.is_empty());
    assert!(
        answer
            .retrieval
            .limitations
            .iter()
            .any(|l| l.contains("No Customer"))
    );
    assert!(
        ai.syntheses.lock().unwrap().is_empty(),
        "nothing to phrase, nothing invented"
    );
}

#[tokio::test]
async fn unsupported_questions_are_explained_not_answered() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::new(json!({"operation": "unsupported",
        "reason": "Guilt is not a graph operation."})));
    let service = f.service(ai.clone(), GraphRagConfig::default());
    let answer = service
        .ask(
            f.graph_id,
            question("Is Alice committing fraud?", AskScope::BoundedGraph),
        )
        .await
        .unwrap();
    assert_eq!(answer.status, AnswerStatus::InsufficientEvidence);
    assert_eq!(answer.source, AnswerSource::Unsupported);
    assert_eq!(
        answer.retrieval.plan.reason.as_deref(),
        Some("Guilt is not a graph operation.")
    );
    assert!(ai.syntheses.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bounded_discovery_that_hits_a_limit_is_partial() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::new(json!({"operation": "shared_neighbor",
        "entity_type": "Customer", "shared_entity_type": "Device"})));
    let config = GraphRagConfig {
        limits: GraphRagLimits {
            max_candidates: 1,
            ..GraphRagLimits::default()
        },
        ..GraphRagConfig::default()
    };
    let answer = f
        .service(ai.clone(), config)
        .ask(
            f.graph_id,
            question("Find customers sharing a device", AskScope::BoundedGraph),
        )
        .await
        .unwrap();
    assert_eq!(answer.status, AnswerStatus::Partial);
    assert!(answer.retrieval.truncated);
    assert_eq!(answer.retrieval.candidates_inspected, 1);
    let synthesis = ai.syntheses.lock().unwrap()[0].clone();
    assert!(
        !synthesis.retrieval.complete,
        "the model is told the scope was bounded"
    );
    assert_eq!(synthesis.context["retrieval"]["complete"], json!(false));
}

#[tokio::test]
async fn ambiguous_references_are_not_resolved_arbitrarily() {
    let f = fixture().await;
    // "the customer" matches three customers.
    let ai = Arc::new(FakeAi::new(json!({"operation": "neighbors",
        "source": {"entity_type": "Customer"}})));
    let answer = f
        .service(ai, GraphRagConfig::default())
        .ask(
            f.graph_id,
            question("What does the customer use?", AskScope::BoundedGraph),
        )
        .await
        .unwrap();
    assert_eq!(
        answer.status,
        AnswerStatus::InsufficientEvidence,
        "even if the model answers"
    );
    assert!(
        answer
            .retrieval
            .limitations
            .iter()
            .any(|l| l.contains("ambiguous"))
    );
    assert!(answer.evidence.relationships().is_empty());
}

#[tokio::test]
async fn fabricated_references_never_become_citations() {
    let f = fixture().await;
    let fabricated = RelationshipId::new().to_string();
    let only_fabricated = fabricated.clone();
    let ai = Arc::new(FakeAi::scripted(
        Ok(shared_device_intent()),
        Box::new(move |_| {
            Ok(SynthesizedAnswer {
                answer: "Alice and Budi share a device [R42] via [P7].".into(),
                evidence_refs: vec![only_fabricated.clone(), "R42".into(), "E99".into()],
                insufficient_evidence: false,
                warnings: vec![],
            })
        }),
    ));
    let answer = f
        .service(ai, GraphRagConfig::default())
        .ask(
            f.graph_id,
            question("Who shares?", AskScope::SelectedEntities(vec![f.alice])),
        )
        .await
        .unwrap();
    assert!(answer.citations.is_empty());
    assert_eq!(
        answer.status,
        AnswerStatus::InsufficientEvidence,
        "no retrieved evidence cited"
    );
    assert!(!answer.answer.contains("R42") && !answer.answer.contains("P7"));
    assert!(
        answer
            .warnings
            .iter()
            .any(|w| w.contains("Removed 5 citation"))
    );
    assert!(!answer.answer.contains(&fabricated));

    // Mixed: valid references stay, invented ones go.
    let ai = Arc::new(FakeAi::scripted(
        Ok(shared_device_intent()),
        Box::new(|_| {
            Ok(SynthesizedAnswer {
                answer: "Alice [E1] and Budi share it [E1, R9].".into(),
                evidence_refs: vec!["E1".into(), "R9".into()],
                insufficient_evidence: false,
                warnings: vec!["w".repeat(1_000)],
            })
        }),
    ));
    let answer = f
        .service(ai, GraphRagConfig::default())
        .ask(
            f.graph_id,
            question("Who shares?", AskScope::SelectedEntities(vec![f.alice])),
        )
        .await
        .unwrap();
    assert_eq!(answer.status, AnswerStatus::Answered);
    assert_eq!(answer.answer, "Alice [E1] and Budi share it [E1].");
    assert_eq!(answer.citations.len(), 1);
    assert!(
        answer.warnings[0].chars().count() <= 301,
        "model warnings are bounded"
    );
}

#[tokio::test]
async fn the_model_may_judge_evidence_insufficient() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::scripted(
        Ok(shared_device_intent()),
        Box::new(|_| {
            Ok(SynthesizedAnswer {
                answer: "A shared device [E1] does not establish fraud.".into(),
                evidence_refs: vec!["E1".into()],
                insufficient_evidence: true,
                warnings: vec![],
            })
        }),
    ));
    let answer = f
        .service(ai, GraphRagConfig::default())
        .ask(
            f.graph_id,
            question(
                "Is Alice definitely committing fraud?",
                AskScope::SelectedEntities(vec![f.alice]),
            ),
        )
        .await
        .unwrap();
    assert_eq!(answer.status, AnswerStatus::InsufficientEvidence);
    assert_eq!(answer.citations.len(), 1);
}

#[tokio::test]
async fn invalid_plans_and_ai_failures_map_to_stable_errors() {
    let f = fixture().await;
    type Expectation = fn(&ApplicationError) -> bool;
    let cases: Vec<(Result<Value, AiError>, Expectation)> = vec![
        (
            Ok(json!({"operation": "neighbors", "source": {"entity_type": "Merchant"}})),
            |e| {
                matches!(
                    e,
                    ApplicationError::GraphRag(GraphRagError::InvalidRetrievalPlan(_))
                )
            },
        ),
        (
            Ok(json!({"operation": "neighbors", "source": {"ref": "S1"}, "max_depth": 5})),
            |e| {
                matches!(
                    e,
                    ApplicationError::GraphRag(GraphRagError::RetrievalLimitExceeded(_))
                )
            },
        ),
        (Err(AiError::NotConfigured), |e| {
            matches!(
                e,
                ApplicationError::GraphRag(GraphRagError::AiNotConfigured)
            )
        }),
        (Err(AiError::Unavailable("down".into())), |e| {
            matches!(e, ApplicationError::GraphRag(GraphRagError::AiUnavailable))
        }),
        (Err(AiError::RateLimited), |e| {
            matches!(e, ApplicationError::GraphRag(GraphRagError::AiUnavailable))
        }),
        (Err(AiError::Timeout), |e| {
            matches!(e, ApplicationError::GraphRag(GraphRagError::AiTimeout))
        }),
        (Err(AiError::InvalidResponse("bad".into())), |e| {
            matches!(
                e,
                ApplicationError::GraphRag(GraphRagError::InvalidRetrievalPlan(_))
            )
        }),
    ];
    for (intent, expected) in cases {
        let ai = Arc::new(FakeAi::scripted(intent.clone(), Box::new(cite_everything)));
        let result = f
            .service(ai, GraphRagConfig::default())
            .ask(
                f.graph_id,
                question("Who?", AskScope::SelectedEntities(vec![f.alice])),
            )
            .await;
        assert!(
            expected(result.as_ref().unwrap_err()),
            "{intent:?} → {result:?}"
        );
    }

    let ai = Arc::new(FakeAi::scripted(
        Ok(shared_device_intent()),
        Box::new(|_| Err(AiError::InvalidResponse("bad".into()))),
    ));
    assert!(matches!(
        f.service(ai, GraphRagConfig::default())
            .ask(
                f.graph_id,
                question("Who?", AskScope::SelectedEntities(vec![f.alice]))
            )
            .await,
        Err(ApplicationError::GraphRag(GraphRagError::AiInvalidResponse))
    ));

    let disabled = f.service(Arc::new(DisabledGraphRagAi), GraphRagConfig::default());
    assert_eq!(disabled.status().await, AiStatus::NotConfigured);
    assert!(matches!(
        disabled
            .ask(f.graph_id, question("Who?", AskScope::BoundedGraph))
            .await,
        Err(ApplicationError::GraphRag(GraphRagError::AiNotConfigured))
    ));
}

#[tokio::test]
async fn slow_questions_hit_the_deadline() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::new(shared_device_intent()).slow(Duration::from_secs(5)));
    let config = GraphRagConfig {
        timeout: Duration::from_secs(1),
        ..GraphRagConfig::default()
    };
    assert!(matches!(
        f.service(ai, config)
            .ask(
                f.graph_id,
                question("Who?", AskScope::SelectedEntities(vec![f.alice]))
            )
            .await,
        Err(ApplicationError::GraphRag(GraphRagError::Timeout(1)))
    ));
}

#[tokio::test]
async fn concurrent_questions_respect_the_limit() {
    let f = fixture().await;
    let ai = Arc::new(FakeAi::new(shared_device_intent()).slow(Duration::from_millis(100)));
    let service = f.service(
        ai.clone(),
        GraphRagConfig {
            concurrency: 1,
            ..GraphRagConfig::default()
        },
    );
    let ask = || {
        service.ask(
            f.graph_id,
            question("Who?", AskScope::SelectedEntities(vec![f.alice])),
        )
    };
    let (first, second, third) = tokio::join!(ask(), ask(), ask());
    for answer in [first, second, third] {
        assert_eq!(answer.unwrap().status, AnswerStatus::Answered);
    }
    assert_eq!(ai.max_in_flight.load(Ordering::SeqCst), 1);
}
