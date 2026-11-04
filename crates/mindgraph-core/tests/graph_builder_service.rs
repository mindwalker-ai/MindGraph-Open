//! Use-case tests of the AI Graph Builder with a scripted fake AI and
//! in-memory stores (no AI service, provider, database or network).

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mindgraph_core::{
    AiError, AiMetadata, AiStatus, AiSuggestion, ApplicationError, CreateGraphProject,
    GraphBuilderAi, GraphBuilderConfig, GraphBuilderRequest, GraphBuilderService,
    GraphBuilderSuggestion, GraphProjectService, IngestionConfig, IngestionJob, IngestionPorts,
    IngestionService, JobKind, JobStatus, MemoryMetadataStore, MetadataStore, OntologyDefinition,
    OntologyService, PROCESS_RESTARTED,
};
use mindgraph_domain::{DataSourceId, GraphId, IngestionJobId, OntologyVersion, TypeName};
use mindgraph_graph::{EntitySearch, GraphStore, MemoryGraphStore};
use mindgraph_ingestion::MemorySourceStore;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

const CUSTOMERS: &str = "customer_id,name,email\nC001,Ann,ann@example.com\nC002,Bob,\n";
const ACCOUNTS: &str = "account_id,customer_id,balance\nA1,C001,10.5\nA2,C001,3\nA3,C002,7\n";

/// A scripted [`GraphBuilderAi`]: returns queued responses in order and
/// records every request.
struct FakeAi {
    status: Mutex<AiStatus>,
    responses: Mutex<VecDeque<Result<AiSuggestion, AiError>>>,
    requests: Mutex<Vec<GraphBuilderRequest>>,
    gate: Option<Arc<Semaphore>>,
}

impl FakeAi {
    fn ready() -> Self {
        Self {
            status: Mutex::new(AiStatus::Ready {
                provider: "fake".into(),
                model: "fake-model".into(),
            }),
            responses: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
            gate: None,
        }
    }

    fn respond(&self, suggestion: GraphBuilderSuggestion) {
        self.responses.lock().unwrap().push_back(Ok(AiSuggestion {
            suggestion,
            metadata: AiMetadata {
                prompt_version: "graph_builder_v1".into(),
                provider: "fake".into(),
                model: "fake-model".into(),
            },
        }));
    }

    fn fail(&self, error: AiError) {
        self.responses.lock().unwrap().push_back(Err(error));
    }

    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn request(&self, index: usize) -> GraphBuilderRequest {
        self.requests.lock().unwrap()[index].clone()
    }
}

#[async_trait]
impl GraphBuilderAi for FakeAi {
    async fn status(&self) -> AiStatus {
        self.status.lock().unwrap().clone()
    }

    async fn suggest(&self, request: &GraphBuilderRequest) -> Result<AiSuggestion, AiError> {
        if let Some(gate) = &self.gate {
            gate.acquire().await.expect("gate open").forget();
        }
        self.requests.lock().unwrap().push(request.clone());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(AiError::Internal("no scripted response".into())))
    }
}

struct App {
    metadata: Arc<dyn MetadataStore>,
    graph: Arc<MemoryGraphStore>,
    ai: Arc<FakeAi>,
    graphs: GraphProjectService,
    ontology: OntologyService,
    ingestion: IngestionService,
    builder: GraphBuilderService,
    graph_id: GraphId,
}

async fn app_with(ai: FakeAi, config: GraphBuilderConfig) -> App {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let graph = Arc::new(MemoryGraphStore::new());
    let graphs = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata: metadata.clone(),
            graph: graph.clone(),
            sources: Arc::new(MemorySourceStore::new()),
        },
        graphs.workspace_id(),
        IngestionConfig::default(),
    )
    .unwrap();
    let ai = Arc::new(ai);
    let builder = GraphBuilderService::new(ai.clone(), &ingestion, config).unwrap();
    let graph_id = graphs
        .create_graph(CreateGraphProject {
            name: "Fraud".into(),
            description: None,
        })
        .await
        .unwrap()
        .id();
    App {
        ontology: OntologyService::new(metadata.clone(), graphs.workspace_id()),
        metadata,
        graph,
        ai,
        graphs,
        ingestion,
        builder,
        graph_id,
    }
}

async fn app() -> App {
    app_with(FakeAi::ready(), GraphBuilderConfig::default()).await
}

async fn wait(app: &App, id: IngestionJobId) -> IngestionJob {
    for _ in 0..500 {
        let job = app.ingestion.get_job(id).await.unwrap();
        if job.status.is_terminal() {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("job {id} did not finish");
}

async fn upload(app: &App, filename: &str, body: &str) -> DataSourceId {
    let mut session = app
        .ingestion
        .begin_upload(app.graph_id, filename)
        .await
        .unwrap();
    session.write_chunk(body.as_bytes()).await.unwrap();
    session.finish().await.unwrap().id()
}

async fn analysed(app: &App, filename: &str, body: &str) -> DataSourceId {
    let id = upload(app, filename, body).await;
    let job = app
        .ingestion
        .start_analysis(app.graph_id, id)
        .await
        .unwrap();
    assert_eq!(wait(app, job.id).await.status, JobStatus::Completed);
    id
}

fn suggestion(value: Value) -> GraphBuilderSuggestion {
    serde_json::from_value(value).unwrap()
}

/// A good first-dataset suggestion for `customers.csv`.
fn customers_suggestion() -> Value {
    json!({
        "ontology": {
            "entity_types": [{
                "name": "Customer",
                "display_name": "Customer",
                "properties": [
                    {"name": "customer_id", "type": "string", "required": true},
                    {"name": "name", "type": "string", "required": false},
                    {"name": "email", "type": "string", "required": false}
                ]
            }],
            "relationship_types": []
        },
        "mappings": [{"dataset": "customers.csv", "mapping": {
                "entities": [{
                    "alias": "customer",
                    "entity_type": "Customer",
                    "mode": "create_or_match",
                    "identity": [{"source_field": "customer_id", "property": "customer_id"}],
                    "properties": [
                        {"source_field": "customer_id", "property": "customer_id"},
                        {"source_field": "name", "property": "name"},
                        {"source_field": "email", "property": "email"}
                    ]
                }],
                "relationships": []
            }}],
        "confidence": 0.93,
        "annotations": {
            "entities": [{"name": "Customer", "confidence": 0.97,
                          "reason": "customer_id identifies each record", "existing": true}],
            "relationships": []
        },
        "warnings": ["email is empty for some records"]
    })
}

/// The second-dataset suggestion: existing Customer, new Account, OWNS.
fn accounts_suggestion() -> Value {
    json!({
        "ontology": {
            "entity_types": [
                {"name": "Customer", "display_name": "Customer", "properties": [
                    {"name": "customer_id", "type": "string", "required": true},
                    {"name": "name", "type": "string", "required": false},
                    {"name": "email", "type": "string", "required": false}
                ]},
                {"name": "Account", "properties": [
                    {"name": "account_id", "type": "string", "required": true},
                    {"name": "balance", "type": "float", "required": false}
                ]}
            ],
            "relationship_types": [
                {"name": "OWNS", "source_entity_type": "Customer", "target_entity_type": "Account"}
            ]
        },
        "mappings": [{"dataset": "accounts.csv", "mapping": {
                "entities": [
                    {"alias": "customer", "entity_type": "Customer", "mode": "match_only",
                     "identity": [{"source_field": "customer_id", "property": "customer_id"}],
                     "properties": []},
                    {"alias": "account", "entity_type": "Account", "mode": "create_or_match",
                     "identity": [{"source_field": "account_id", "property": "account_id"}],
                     "properties": [
                        {"source_field": "account_id", "property": "account_id"},
                        {"source_field": "balance", "property": "balance"}
                     ]}
                ],
                "relationships": [
                    {"relationship_type": "OWNS", "source": "customer", "target": "account"}
                ]
            }}],
        "confidence": 0.9,
        "annotations": {
            "entities": [
                {"name": "Customer", "confidence": 0.95, "reason": "customer_id references the existing Customer"},
                {"name": "Account", "confidence": 0.94, "reason": "account_id identifies each record"},
                {"name": "Merchant", "confidence": 0.2, "reason": "not part of the ontology"}
            ],
            "relationships": [
                {"name": "OWNS", "confidence": 0.91, "reason": "each account row names its customer"}
            ]
        },
        "warnings": []
    })
}

/// Applies a reviewed suggestion through the M4 services, as the UI does.
async fn generate(
    app: &App,
    source: DataSourceId,
    suggestion: &GraphBuilderSuggestion,
) -> IngestionJob {
    app.ontology
        .create_version(app.graph_id, &suggestion.ontology)
        .await
        .unwrap();
    let job = app
        .ingestion
        .start_ingestion(
            app.graph_id,
            source,
            &format!("ingest-{source}"),
            suggestion.mappings[0].mapping.clone(),
        )
        .await
        .unwrap();
    wait(app, job.id).await
}

async fn count(app: &App, entity_type: &str) -> usize {
    let search = EntitySearch::new()
        .with_entity_type(TypeName::new(entity_type).unwrap())
        .with_limit(1_000)
        .unwrap();
    app.graph
        .search_entities(app.graph_id, &search)
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn first_dataset_suggestion_is_validated_and_generates_a_graph() {
    let app = app().await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    app.ai.respond(suggestion(customers_suggestion()));

    let job = app
        .builder
        .start_suggestion(app.graph_id, &[source], "suggest-1")
        .await
        .unwrap();
    assert_eq!(
        (job.kind, job.status),
        (JobKind::AiSuggest, JobStatus::Queued)
    );
    assert_eq!(
        job.ontology_version,
        Some(OntologyVersion::INITIAL),
        "pinned"
    );
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Completed, "{:?}", done.error);
    assert!((done.progress - 1.0).abs() < f64::EPSILON);

    let result = done.graph_builder_result().unwrap();
    assert_eq!(result.prompt_version, "graph_builder_v1");
    assert_eq!(
        (result.ontology_version_base, result.repair_attempts),
        (1, 0)
    );
    let customer = &result.suggestion.annotations.entities[0];
    assert!(
        !customer.existing,
        "Rust sets `existing`, ignoring the AI's claim"
    );

    // What the AI received: the bounded profile and the pinned ontology.
    let request = app.ai.request(0);
    assert_eq!(request.correlation_id, job.id.to_string());
    assert_eq!(request.datasets.len(), 1);
    assert_eq!(request.datasets[0].name, "customers.csv");
    assert_eq!(
        request.datasets[0].profile.field_names(),
        ["customer_id", "name", "email"]
    );
    assert_eq!(
        result.suggestion.mappings[0].data_source_id,
        Some(source),
        "Rust links each mapping to its data source"
    );
    assert_eq!(request.active_ontology.version, 1);
    assert!(request.active_ontology.definition.entity_types.is_empty());
    assert!(request.repair.is_none());

    // Reviewed as-is, it runs through the unchanged M4 pipeline.
    let ingested = generate(&app, source, &result.suggestion).await;
    assert_eq!(
        ingested.status,
        JobStatus::Completed,
        "{:?}",
        ingested.error
    );
    assert_eq!(count(&app, "Customer").await, 2);
}

#[tokio::test]
async fn second_dataset_reuses_existing_customers_with_match_only() {
    let app = app().await;
    let customers = analysed(&app, "customers.csv", CUSTOMERS).await;
    app.ai.respond(suggestion(customers_suggestion()));
    let first = app
        .builder
        .start_suggestion(app.graph_id, &[customers], "s1")
        .await
        .unwrap();
    let first = wait(&app, first.id).await;
    generate(
        &app,
        customers,
        &first.graph_builder_result().unwrap().suggestion,
    )
    .await;

    let accounts = analysed(&app, "accounts.csv", ACCOUNTS).await;
    app.ai.respond(suggestion(accounts_suggestion()));
    let job = app
        .builder
        .start_suggestion(app.graph_id, &[accounts], "s2")
        .await
        .unwrap();
    assert_eq!(job.ontology_version, Some(OntologyVersion::new(2).unwrap()));
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Completed, "{:?}", done.error);
    let result = done.graph_builder_result().unwrap();

    let request = app.ai.request(1);
    let existing: Vec<&str> = request
        .active_ontology
        .definition
        .entity_types
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(existing, ["Customer"], "the AI sees the existing ontology");

    let annotations = &result.suggestion.annotations;
    let flags: Vec<(&str, bool)> = annotations
        .entities
        .iter()
        .map(|a| (a.name.as_str(), a.existing))
        .collect();
    assert_eq!(
        flags,
        [("Customer", true), ("Account", false)],
        "unknown Merchant dropped"
    );
    assert!(!annotations.relationships[0].existing);

    let ingested = generate(&app, accounts, &result.suggestion).await;
    assert_eq!(
        ingested.status,
        JobStatus::Completed,
        "{:?}",
        ingested.error
    );
    let summary = ingested.ingestion_summary().unwrap();
    assert_eq!(summary.entities_matched, 2, "existing customers are reused");
    assert_eq!(
        (summary.entities_created, summary.relationships_created),
        (3, 3)
    );
    assert_eq!(count(&app, "Customer").await, 2, "no duplicate customers");
}

#[tokio::test]
async fn preflight_rejects_bad_requests_before_a_job_or_provider_call() {
    let app = app().await;
    let pending = upload(&app, "customers.csv", CUSTOMERS).await;
    assert!(matches!(
        app.builder
            .start_suggestion(app.graph_id, &[pending], "k")
            .await,
        Err(ApplicationError::Conflict(_))
    ));
    assert!(matches!(
        app.builder
            .start_suggestion(app.graph_id, &[DataSourceId::new()], "k")
            .await,
        Err(ApplicationError::DataSourceNotFound(_))
    ));
    assert!(matches!(
        app.builder
            .start_suggestion(GraphId::new(), &[pending], "k")
            .await,
        Err(ApplicationError::GraphNotFound(_))
    ));
    for key in ["", "has space"] {
        assert!(matches!(
            app.builder
                .start_suggestion(app.graph_id, &[pending], key)
                .await,
            Err(ApplicationError::InvalidRequest(_))
        ));
    }
    assert_eq!(app.ai.calls(), 0);
    assert!(
        app.metadata
            .find_job_by_idempotency_key(app.graph_id, "k")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn sources_of_another_graph_are_not_found() {
    let app = app().await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    let other = app
        .graphs
        .create_graph(CreateGraphProject {
            name: "Other".into(),
            description: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        app.builder
            .start_suggestion(other.id(), &[source], "k")
            .await,
        Err(ApplicationError::DataSourceNotFound(_))
    ));
}

#[tokio::test]
async fn disabled_or_unreachable_ai_is_reported_before_scheduling() {
    let app = app().await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    *app.ai.status.lock().unwrap() = AiStatus::NotConfigured;
    assert!(matches!(
        app.builder
            .start_suggestion(app.graph_id, &[source], "k")
            .await,
        Err(ApplicationError::AiNotConfigured)
    ));
    *app.ai.status.lock().unwrap() = AiStatus::Unavailable;
    assert!(matches!(
        app.builder
            .start_suggestion(app.graph_id, &[source], "k")
            .await,
        Err(ApplicationError::AiUnavailable)
    ));
    assert_eq!(app.ai.calls(), 0);
}

#[tokio::test]
async fn idempotency_keys_return_the_same_job_without_another_provider_call() {
    let app = app().await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    app.ai.respond(suggestion(customers_suggestion()));
    let first = app
        .builder
        .start_suggestion(app.graph_id, &[source], "same")
        .await
        .unwrap();
    wait(&app, first.id).await;
    let again = app
        .builder
        .start_suggestion(app.graph_id, &[source], "same")
        .await
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(app.ai.calls(), 1, "no second provider call");

    // Same key, other data source: conflict.
    let other_source = analysed(&app, "more.csv", CUSTOMERS).await;
    assert!(matches!(
        app.builder
            .start_suggestion(app.graph_id, &[other_source], "same")
            .await,
        Err(ApplicationError::Conflict(_))
    ));

    // Same key as an ingestion job: conflict.
    let reviewed = suggestion(customers_suggestion());
    app.ontology
        .create_version(app.graph_id, &reviewed.ontology)
        .await
        .unwrap();
    app.ingestion
        .start_ingestion(
            app.graph_id,
            source,
            "ingest-key",
            reviewed.mappings[0].mapping.clone(),
        )
        .await
        .unwrap();
    assert!(matches!(
        app.builder
            .start_suggestion(app.graph_id, &[source], "ingest-key")
            .await,
        Err(ApplicationError::Conflict(_))
    ));
    assert_eq!(app.ai.calls(), 1);
}

#[tokio::test]
async fn invalid_suggestions_fail_after_one_repair_attempt() {
    let app = app().await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    let mut bad_name = customers_suggestion();
    bad_name["ontology"]["entity_types"][0]["name"] = json!("Customer Record");
    let mut bad_mapping = customers_suggestion();
    bad_mapping["mappings"][0]["mapping"]["entities"][0]["properties"][1]["source_field"] =
        json!("full_name");
    app.ai.respond(suggestion(bad_name));
    app.ai.respond(suggestion(bad_mapping));

    let job = app
        .builder
        .start_suggestion(app.graph_id, &[source], "k")
        .await
        .unwrap();
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Failed);
    let error = done.error.unwrap();
    assert_eq!(error.code, "AI_VALIDATION_FAILED");
    assert!(error.message.contains("full_name"), "{}", error.message);
    assert!(done.result.is_none());
    assert_eq!(app.ai.calls(), 2, "one call plus exactly one repair");

    let repair = app.ai.request(1).repair.unwrap();
    assert!(
        repair.errors[0].contains("invalid ontology"),
        "{:?}",
        repair.errors
    );
    assert_eq!(
        repair.previous.ontology.entity_types[0].name,
        "Customer Record"
    );
}

#[tokio::test]
async fn a_repair_can_fix_a_suggestion() {
    let app = app().await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    let mut unmapped_identity = customers_suggestion();
    unmapped_identity["mappings"][0]["mapping"]["entities"][0]["properties"] =
        json!([{"source_field": "name", "property": "name"}]);
    app.ai.respond(suggestion(unmapped_identity));
    app.ai.respond(suggestion(customers_suggestion()));

    let job = app
        .builder
        .start_suggestion(app.graph_id, &[source], "k")
        .await
        .unwrap();
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Completed, "{:?}", done.error);
    assert_eq!(done.graph_builder_result().unwrap().repair_attempts, 1);
    let repair = app.ai.request(1).repair.unwrap();
    assert!(
        repair.errors.iter().any(|e| e.contains("customer_id")),
        "{:?}",
        repair.errors
    );
}

#[tokio::test]
async fn zero_repair_attempts_fail_immediately() {
    let app = app_with(
        FakeAi::ready(),
        GraphBuilderConfig {
            max_repair_attempts: 0,
            ..GraphBuilderConfig::default()
        },
    )
    .await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    let mut bad = customers_suggestion();
    bad["confidence"] = json!(1.5);
    app.ai.respond(suggestion(bad));
    let job = app
        .builder
        .start_suggestion(app.graph_id, &[source], "k")
        .await
        .unwrap();
    let done = wait(&app, job.id).await;
    assert_eq!(done.error.unwrap().code, "AI_VALIDATION_FAILED");
    assert_eq!(app.ai.calls(), 1);
}

#[tokio::test]
async fn destructive_changes_to_the_existing_ontology_are_rejected() {
    let app = app().await;
    app.ontology
        .create_version(app.graph_id, &suggestion(accounts_suggestion()).ontology)
        .await
        .unwrap();
    let accounts = analysed(&app, "accounts.csv", ACCOUNTS).await;

    let mut cases: Vec<(Value, &str)> = Vec::new();
    let mut removed = accounts_suggestion();
    removed["ontology"]["relationship_types"] = json!([]);
    removed["mappings"][0]["mapping"]["relationships"] = json!([]);
    cases.push((removed, "existing relationship type `OWNS` must be kept"));
    let mut retyped = accounts_suggestion();
    retyped["ontology"]["entity_types"][1]["properties"][1]["type"] = json!("integer");
    cases.push((retyped, "must keep type float"));
    let mut now_required = accounts_suggestion();
    now_required["ontology"]["entity_types"][0]["properties"][1]["required"] = json!(true);
    cases.push((now_required, "must stay optional"));
    let mut new_required = accounts_suggestion();
    new_required["ontology"]["entity_types"][1]["properties"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "iban", "type": "string", "required": true}));
    cases.push((new_required, "must be optional"));
    let mut rewired = accounts_suggestion();
    rewired["ontology"]["relationship_types"][0]["source_entity_type"] = json!("Account");
    rewired["ontology"]["relationship_types"][0]["target_entity_type"] = json!("Customer");
    cases.push((rewired, "must keep its endpoints"));

    for (index, (case, expected)) in cases.into_iter().enumerate() {
        // The same destructive answer to the call and to the repair.
        app.ai.respond(suggestion(case.clone()));
        app.ai.respond(suggestion(case));
        let job = app
            .builder
            .start_suggestion(app.graph_id, &[accounts], &format!("destructive-{index}"))
            .await
            .unwrap();
        let error = wait(&app, job.id).await.error.expect("rejected");
        assert_eq!(error.code, "AI_VALIDATION_FAILED");
        assert!(
            error.message.contains(expected),
            "{expected}: {}",
            error.message
        );
    }
}

#[tokio::test]
async fn provider_failures_fail_the_job_with_their_category() {
    let app = app().await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    for (index, (error, code)) in [
        (AiError::Timeout, "AI_TIMEOUT"),
        (AiError::RateLimited, "AI_RATE_LIMITED"),
        (
            AiError::ProviderRejected("401".into()),
            "AI_PROVIDER_REJECTED",
        ),
        (
            AiError::InvalidResponse("not JSON".into()),
            "AI_INVALID_RESPONSE",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        app.ai.fail(error);
        let job = app
            .builder
            .start_suggestion(app.graph_id, &[source], &format!("fail-{index}"))
            .await
            .unwrap();
        let done = wait(&app, job.id).await;
        assert_eq!(done.status, JobStatus::Failed);
        assert_eq!(done.error.unwrap().code, code);
    }
}

#[tokio::test]
async fn suggestions_keep_their_pinned_ontology_version() {
    let gate = Arc::new(Semaphore::new(0));
    let app = app_with(
        FakeAi {
            gate: Some(gate.clone()),
            ..FakeAi::ready()
        },
        GraphBuilderConfig::default(),
    )
    .await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    app.ai.respond(suggestion(customers_suggestion()));
    let job = app
        .builder
        .start_suggestion(app.graph_id, &[source], "k")
        .await
        .unwrap();
    // While the AI "thinks", someone activates version 2 with other types.
    app.ontology
        .create_version(
            app.graph_id,
            &serde_json::from_value::<OntologyDefinition>(json!({
                "entity_types": [{"name": "Device", "properties": []}]
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    gate.add_permits(1);
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Completed, "{:?}", done.error);
    assert_eq!(
        done.graph_builder_result().unwrap().ontology_version_base,
        1
    );
    assert_eq!(done.ontology_version, Some(OntologyVersion::INITIAL));
}

#[tokio::test]
async fn slow_ai_jobs_do_not_hold_up_dataset_jobs() {
    let gate = Arc::new(Semaphore::new(0));
    let app = app_with(
        FakeAi {
            gate: Some(gate.clone()),
            ..FakeAi::ready()
        },
        GraphBuilderConfig::default(),
    )
    .await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    // As many stuck AI jobs as there are dataset workers (both default to 2).
    let mut stuck = Vec::new();
    for key in ["a", "b"] {
        app.ai.respond(suggestion(customers_suggestion()));
        stuck.push(
            app.builder
                .start_suggestion(app.graph_id, &[source], key)
                .await
                .unwrap(),
        );
    }
    // Analysis still runs: AI jobs have their own workers.
    analysed(&app, "more.csv", CUSTOMERS).await;
    for job in &stuck {
        let current = app.ingestion.get_job(job.id).await.unwrap();
        assert!(!current.status.is_terminal(), "{:?}", current.status);
    }
    gate.add_permits(2);
    for job in stuck {
        assert_eq!(wait(&app, job.id).await.status, JobStatus::Completed);
    }
}

#[tokio::test]
async fn restart_recovery_includes_ai_jobs() {
    let app = app().await;
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    let job = IngestionJob::graph_builder(
        app.graph_id,
        vec![source],
        OntologyVersion::INITIAL,
        "stuck".into(),
        chrono::Utc::now(),
    );
    app.metadata.create_job(&job).await.unwrap();
    app.metadata.start_job(job.id).await.unwrap();
    assert_eq!(app.ingestion.recover_incomplete_jobs().await.unwrap(), 1);
    let recovered = app.ingestion.get_job(job.id).await.unwrap();
    assert_eq!(recovered.status, JobStatus::Failed);
    assert_eq!(recovered.error.unwrap().code, PROCESS_RESTARTED);
}

#[tokio::test]
async fn too_many_repair_attempts_are_rejected() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata,
            graph: Arc::new(MemoryGraphStore::new()),
            sources: Arc::new(MemorySourceStore::new()),
        },
        mindgraph_domain::WorkspaceId::new(),
        IngestionConfig::default(),
    )
    .unwrap();
    let result = GraphBuilderService::new(
        Arc::new(FakeAi::ready()),
        &ingestion,
        GraphBuilderConfig {
            max_repair_attempts: 3,
            ..GraphBuilderConfig::default()
        },
    );
    assert!(matches!(result, Err(ApplicationError::InvalidRequest(_))));
}

/// One suggestion for both files, listed in the wrong order: `accounts.csv`
/// only matches the customers that `customers.csv` creates.
fn both_datasets_suggestion() -> Value {
    let mut both = accounts_suggestion();
    let customers = customers_suggestion()["mappings"][0].clone();
    both["mappings"].as_array_mut().unwrap().push(customers);
    both
}

#[tokio::test]
async fn several_datasets_are_modelled_together_in_a_safe_order() {
    let app = app().await;
    let accounts = analysed(&app, "accounts.csv", ACCOUNTS).await;
    let customers = analysed(&app, "customers.csv", CUSTOMERS).await;
    app.ai.respond(suggestion(both_datasets_suggestion()));
    let job = app
        .builder
        .start_suggestion(app.graph_id, &[accounts, customers], "both")
        .await
        .unwrap();
    assert_eq!(job.data_source_ids, [accounts, customers]);
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Completed, "{:?}", done.error);
    assert_eq!(done.data_source_ids, [accounts, customers]);

    let request = app.ai.request(0);
    let names: Vec<&str> = request.datasets.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        ["accounts.csv", "customers.csv"],
        "the client's order"
    );

    // Creators first: customers.csv creates the Customer accounts.csv matches.
    let mappings = &done.graph_builder_result().unwrap().suggestion.mappings;
    let order: Vec<(&str, Option<DataSourceId>)> = mappings
        .iter()
        .map(|m| (m.dataset.as_str(), m.data_source_id))
        .collect();
    assert_eq!(
        order,
        [
            ("customers.csv", Some(customers)),
            ("accounts.csv", Some(accounts))
        ]
    );

    // Applied in that order, the cross-file relationship resolves.
    let suggestion = &done.graph_builder_result().unwrap().suggestion;
    app.ontology
        .create_version(app.graph_id, &suggestion.ontology)
        .await
        .unwrap();
    for mapping in &suggestion.mappings {
        let source = mapping.data_source_id.unwrap();
        let job = app
            .ingestion
            .start_ingestion(
                app.graph_id,
                source,
                &format!("ingest-{source}"),
                mapping.mapping.clone(),
            )
            .await
            .unwrap();
        let job = wait(&app, job.id).await;
        assert_eq!(job.status, JobStatus::Completed, "{:?}", job.error);
    }
    assert_eq!(count(&app, "Customer").await, 2);
    assert_eq!(count(&app, "Account").await, 3);
}

#[tokio::test]
async fn every_dataset_needs_exactly_one_mapping() {
    let app = app_with(
        FakeAi::ready(),
        GraphBuilderConfig {
            max_repair_attempts: 0,
            ..GraphBuilderConfig::default()
        },
    )
    .await;
    let accounts = analysed(&app, "accounts.csv", ACCOUNTS).await;
    let customers = analysed(&app, "customers.csv", CUSTOMERS).await;
    let mut missing = both_datasets_suggestion();
    missing["mappings"].as_array_mut().unwrap().pop();
    let mut unknown = both_datasets_suggestion();
    unknown["mappings"][1]["dataset"] = json!("orders.csv");
    let mut duplicate = both_datasets_suggestion();
    duplicate["mappings"][1]["dataset"] = json!("accounts.csv");
    let mut never_created = both_datasets_suggestion();
    // customers.csv now only matches the Customer that nobody creates.
    never_created["mappings"][1]["mapping"]["entities"][0]["mode"] = json!("match_only");
    never_created["mappings"][1]["mapping"]["entities"][0]["properties"] = json!([]);
    for (index, (bad, expected)) in [
        (missing, "dataset `customers.csv` needs a mapping"),
        (unknown, "`orders.csv` is not one of the datasets"),
        (
            duplicate,
            "dataset `accounts.csv` has more than one mapping",
        ),
        (never_created, "entity type `Customer` is only matched"),
    ]
    .into_iter()
    .enumerate()
    {
        app.ai.respond(suggestion(bad));
        let job = app
            .builder
            .start_suggestion(
                app.graph_id,
                &[accounts, customers],
                &format!("bad-{index}"),
            )
            .await
            .unwrap();
        let done = wait(&app, job.id).await;
        assert_eq!(done.status, JobStatus::Failed);
        let error = done.error.unwrap();
        assert_eq!(error.code, "AI_VALIDATION_FAILED");
        assert!(error.message.contains(expected), "{}", error.message);
    }
}

#[tokio::test]
async fn data_source_lists_are_validated_and_replayed_exactly() {
    let app = app().await;
    let accounts = analysed(&app, "accounts.csv", ACCOUNTS).await;
    let customers = analysed(&app, "customers.csv", CUSTOMERS).await;
    let too_many = [accounts; 11];
    for ids in [&[][..], &[accounts, accounts][..], &too_many[..]] {
        assert!(matches!(
            app.builder.start_suggestion(app.graph_id, ids, "k").await,
            Err(ApplicationError::InvalidRequest(_))
        ));
    }
    app.ai.respond(suggestion(both_datasets_suggestion()));
    let first = app
        .builder
        .start_suggestion(app.graph_id, &[accounts, customers], "same")
        .await
        .unwrap();
    let again = app
        .builder
        .start_suggestion(app.graph_id, &[accounts, customers], "same")
        .await
        .unwrap();
    assert_eq!(again.id, first.id);
    for other in [&[customers, accounts][..], &[accounts][..]] {
        assert!(matches!(
            app.builder
                .start_suggestion(app.graph_id, other, "same")
                .await,
            Err(ApplicationError::Conflict(_))
        ));
    }
    wait(&app, first.id).await;
}

#[tokio::test]
async fn same_filenames_get_distinct_labels() {
    let app = app().await;
    let a = analysed(&app, "customers.csv", CUSTOMERS).await;
    let b = analysed(&app, "customers.csv", CUSTOMERS).await;
    app.ai.fail(AiError::Timeout);
    let job = app
        .builder
        .start_suggestion(app.graph_id, &[a, b], "twins")
        .await
        .unwrap();
    wait(&app, job.id).await;
    let names: Vec<String> = app
        .ai
        .request(0)
        .datasets
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(names, ["customers.csv", "customers.csv (2)"]);
}
