//! Use-case tests of dataset upload, analysis and ingestion, entirely in
//! memory (no PostgreSQL, HugeGraph, filesystem or AI service).

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mindgraph_core::{
    ApplicationError, CreateGraphProject, EntityTypeInput, GraphProjectService, IngestionConfig,
    IngestionJob, IngestionPorts, IngestionService, JobKind, JobStatus, MemoryMetadataStore,
    MetadataStore, OntologyDefinition, OntologyService, PROCESS_RESTARTED, PropertyInput,
    RelationshipTypeInput,
};
use mindgraph_domain::{
    DataSource, DataSourceId, DataSourceStatus, GraphId, GraphStatus, IngestionJobId,
    OntologyVersion, PropertyType, TypeName, WorkspaceId,
};
use mindgraph_graph::{EntitySearch, GraphStore, MemoryGraphStore};
use mindgraph_ingestion::{
    EntityMapping, EntityMode, FieldMapping, IngestionError, IngestionMapping, MemorySourceStore,
    RelationshipMapping, SourceStore, SourceWriter,
};
use tokio::sync::Semaphore;

const CUSTOMERS: &str = "customer_id,name,age\nC001,Ann,34\nC002,Bob,\nC003,Cy,51\n";
const ACCOUNTS: &str = "account_id,customer_id,balance\nA1,C001,10.5\nA2,C001,3\nA3,C002,7\n";

/// A source store whose `open` can be held back, to make job timing
/// deterministic.
#[derive(Clone)]
struct GatedSources {
    inner: MemorySourceStore,
    gate: Option<Arc<Semaphore>>,
}

#[async_trait]
impl SourceStore for GatedSources {
    fn storage_key(&self, id: DataSourceId) -> String {
        self.inner.storage_key(id)
    }
    async fn create(&self, id: DataSourceId) -> Result<Box<dyn SourceWriter>, IngestionError> {
        self.inner.create(id).await
    }
    async fn open(&self, id: DataSourceId) -> Result<Box<dyn Read + Send>, IngestionError> {
        if let Some(gate) = &self.gate {
            gate.acquire().await.expect("gate open").forget();
        }
        self.inner.open(id).await
    }
    async fn delete(&self, id: DataSourceId) -> Result<(), IngestionError> {
        self.inner.delete(id).await
    }
}

struct App {
    metadata: Arc<dyn MetadataStore>,
    graph: Arc<MemoryGraphStore>,
    sources: MemorySourceStore,
    graphs: GraphProjectService,
    ontology: OntologyService,
    ingestion: IngestionService,
    graph_id: GraphId,
}

async fn app_with(config: IngestionConfig, gate: Option<Arc<Semaphore>>) -> App {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let graph = Arc::new(MemoryGraphStore::new());
    let sources = MemorySourceStore::new();
    let graphs = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata: metadata.clone(),
            graph: graph.clone(),
            sources: Arc::new(GatedSources {
                inner: sources.clone(),
                gate,
            }),
        },
        graphs.workspace_id(),
        config,
    )
    .unwrap();
    let ontology = OntologyService::new(metadata.clone(), graphs.workspace_id());
    let graph_id = graphs
        .create_graph(CreateGraphProject {
            name: "Fraud".into(),
            description: None,
        })
        .await
        .unwrap()
        .id();
    App {
        metadata,
        graph,
        sources,
        graphs,
        ontology,
        ingestion,
        graph_id,
    }
}

async fn app() -> App {
    app_with(IngestionConfig::default(), None).await
}

async fn upload(app: &App, filename: &str, body: &str) -> Result<DataSource, ApplicationError> {
    let mut session = app.ingestion.begin_upload(app.graph_id, filename).await?;
    // Several chunks, like a multipart stream.
    for chunk in body.as_bytes().chunks(7) {
        session.write_chunk(chunk).await?;
    }
    session.finish().await
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

async fn analysed(app: &App, filename: &str, body: &str) -> DataSourceId {
    let source = upload(app, filename, body).await.unwrap();
    let job = app
        .ingestion
        .start_analysis(app.graph_id, source.id())
        .await
        .unwrap();
    assert_eq!(wait(app, job.id).await.status, JobStatus::Completed);
    source.id()
}

fn property(name: &str, property_type: PropertyType, required: bool) -> PropertyInput {
    PropertyInput {
        name: name.into(),
        property_type,
        required,
    }
}

fn fraud_ontology() -> OntologyDefinition {
    OntologyDefinition {
        entity_types: vec![
            EntityTypeInput {
                name: "Customer".into(),
                display_name: None,
                description: None,
                properties: vec![
                    property("customer_id", PropertyType::String, true),
                    property("name", PropertyType::String, false),
                    property("age", PropertyType::Integer, false),
                ],
            },
            EntityTypeInput {
                name: "Account".into(),
                display_name: None,
                description: None,
                properties: vec![
                    property("account_id", PropertyType::String, true),
                    property("balance", PropertyType::Float, false),
                ],
            },
        ],
        relationship_types: vec![RelationshipTypeInput {
            name: "OWNS".into(),
            source_entity_type: "Customer".into(),
            target_entity_type: "Account".into(),
            properties: vec![],
        }],
    }
}

fn field(name: &str) -> FieldMapping {
    FieldMapping {
        source_field: name.into(),
        property: name.into(),
    }
}

fn customer_mapping() -> IngestionMapping {
    IngestionMapping {
        entities: vec![EntityMapping {
            alias: "customer".into(),
            entity_type: "Customer".into(),
            mode: EntityMode::CreateOrMatch,
            identity: vec![field("customer_id")],
            properties: vec![field("customer_id"), field("name"), field("age")],
        }],
        relationships: vec![],
    }
}

fn account_mapping() -> IngestionMapping {
    IngestionMapping {
        entities: vec![
            EntityMapping {
                alias: "account".into(),
                entity_type: "Account".into(),
                mode: EntityMode::CreateOrMatch,
                identity: vec![field("account_id")],
                properties: vec![field("account_id"), field("balance")],
            },
            EntityMapping {
                alias: "customer".into(),
                entity_type: "Customer".into(),
                mode: EntityMode::MatchOnly,
                identity: vec![field("customer_id")],
                properties: vec![],
            },
        ],
        relationships: vec![RelationshipMapping {
            relationship_type: "OWNS".into(),
            source: "customer".into(),
            target: "account".into(),
            properties: vec![],
        }],
    }
}

async fn count(app: &App, graph_id: GraphId, entity_type: &str) -> usize {
    let search = EntitySearch::new()
        .with_entity_type(TypeName::new(entity_type).unwrap())
        .with_limit(1_000)
        .unwrap();
    app.graph
        .search_entities(graph_id, &search)
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn upload_stores_a_pending_data_source() {
    let app = app().await;
    let source = upload(&app, "customers.csv", CUSTOMERS).await.unwrap();
    assert_eq!(source.status(), DataSourceStatus::Pending);
    assert_eq!(source.filename(), "customers.csv");
    assert_eq!(source.size_bytes(), CUSTOMERS.len() as u64);
    assert_eq!(source.content_sha256().len(), 64);
    assert!(app.sources.contains(source.id()));
    let view = app
        .ingestion
        .get_data_source(app.graph_id, source.id())
        .await
        .unwrap();
    assert_eq!(view.data_source, source);
    assert!(view.profile.is_none());
}

#[tokio::test]
async fn upload_rejects_unsupported_too_large_and_empty_files() {
    let app = app_with(
        IngestionConfig {
            max_upload_bytes: 16,
            ..IngestionConfig::default()
        },
        None,
    )
    .await;
    assert!(matches!(
        app.ingestion.begin_upload(app.graph_id, "notes.txt").await,
        Err(ApplicationError::UnsupportedFormat(_))
    ));
    assert!(matches!(
        upload(&app, "big.csv", CUSTOMERS).await,
        Err(ApplicationError::PayloadTooLarge { max_bytes: 16 })
    ));
    assert!(matches!(
        upload(&app, "empty.csv", "").await,
        Err(ApplicationError::InvalidRequest(_))
    ));
    assert!(matches!(
        app.ingestion.begin_upload(GraphId::new(), "a.csv").await,
        Err(ApplicationError::GraphNotFound(_))
    ));
}

#[tokio::test]
async fn upload_keeps_only_the_base_name_of_a_path() {
    let app = app().await;
    let source = upload(&app, "../../etc/customers.csv", CUSTOMERS)
        .await
        .unwrap();
    assert_eq!(source.filename(), "customers.csv");
    let source = upload(&app, "C:\\data\\customers.CSV", CUSTOMERS)
        .await
        .unwrap();
    assert_eq!(source.filename(), "customers.CSV");
}

#[tokio::test]
async fn analysis_profiles_the_data_source() {
    let app = app().await;
    let source = upload(&app, "customers.csv", CUSTOMERS).await.unwrap();
    let job = app
        .ingestion
        .start_analysis(app.graph_id, source.id())
        .await
        .unwrap();
    assert_eq!(
        (job.kind, job.status),
        (JobKind::Analyze, JobStatus::Queued)
    );
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Completed);
    assert!((done.progress - 1.0).abs() < f64::EPSILON);
    assert!(done.result.is_none() && done.error.is_none());
    let view = app
        .ingestion
        .get_data_source(app.graph_id, source.id())
        .await
        .unwrap();
    assert_eq!(view.data_source.status(), DataSourceStatus::Ready);
    let profile = view.profile.unwrap();
    assert_eq!(profile.record_count, 3);
    assert_eq!(profile.field_names(), ["customer_id", "name", "age"]);
    assert_eq!(profile.fields[2].inferred_type, PropertyType::Integer);
    assert!(profile.fields[2].nullable);
}

#[tokio::test]
async fn failed_analysis_marks_the_data_source_failed() {
    let app = app().await;
    let source = upload(&app, "bad.json", r#"[{"a": {"nested": 1}}]"#)
        .await
        .unwrap();
    let job = app
        .ingestion
        .start_analysis(app.graph_id, source.id())
        .await
        .unwrap();
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Failed);
    assert_eq!(done.error.unwrap().code, "UNSUPPORTED_FORMAT");
    let view = app
        .ingestion
        .get_data_source(app.graph_id, source.id())
        .await
        .unwrap();
    assert_eq!(view.data_source.status(), DataSourceStatus::Failed);
    assert!(
        app.sources.contains(source.id()),
        "the file is kept for diagnostics"
    );
}

#[tokio::test]
async fn data_sources_are_scoped_to_their_graph() {
    let app = app().await;
    let source = upload(&app, "customers.csv", CUSTOMERS).await.unwrap();
    let other = app
        .graphs
        .create_graph(CreateGraphProject {
            name: "Other".into(),
            description: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        app.ingestion.get_data_source(other.id(), source.id()).await,
        Err(ApplicationError::DataSourceNotFound(_))
    ));
    assert!(matches!(
        app.ingestion.start_analysis(other.id(), source.id()).await,
        Err(ApplicationError::DataSourceNotFound(_))
    ));
}

#[tokio::test]
async fn ingestion_builds_the_graph_incrementally() {
    let app = app().await;
    app.ontology
        .create_version(app.graph_id, &fraud_ontology())
        .await
        .unwrap();
    assert_eq!(
        app.graphs.get_graph(app.graph_id).await.unwrap().status(),
        GraphStatus::Draft
    );

    let customers = analysed(&app, "customers.csv", CUSTOMERS).await;
    let job = app
        .ingestion
        .start_ingestion(app.graph_id, customers, "customers-1", customer_mapping())
        .await
        .unwrap();
    assert_eq!(
        job.ontology_version,
        Some(OntologyVersion::new(2).unwrap()),
        "pinned"
    );
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Completed, "{:?}", done.error);
    let summary = done.result.unwrap();
    assert_eq!(
        (summary.records_processed, summary.entities_created),
        (3, 3)
    );
    assert_eq!(count(&app, app.graph_id, "Customer").await, 3);
    assert_eq!(
        app.graphs.get_graph(app.graph_id).await.unwrap().status(),
        GraphStatus::Active
    );

    let accounts = analysed(&app, "accounts.csv", ACCOUNTS).await;
    let job = app
        .ingestion
        .start_ingestion(app.graph_id, accounts, "accounts-1", account_mapping())
        .await
        .unwrap();
    let summary = wait(&app, job.id).await.result.unwrap();
    assert_eq!(summary.entities_created, 3, "three accounts");
    assert_eq!(
        summary.entities_matched, 2,
        "C001 and C002, each looked up once"
    );
    assert_eq!(summary.relationships_created, 3);
    assert_eq!(
        count(&app, app.graph_id, "Customer").await,
        3,
        "no duplicate customers"
    );
}

#[tokio::test]
async fn idempotency_keys_return_the_original_job() {
    let app = app().await;
    app.ontology
        .create_version(app.graph_id, &fraud_ontology())
        .await
        .unwrap();
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    let first = app
        .ingestion
        .start_ingestion(app.graph_id, source, "key-1", customer_mapping())
        .await
        .unwrap();
    let again = app
        .ingestion
        .start_ingestion(app.graph_id, source, "key-1", customer_mapping())
        .await
        .unwrap();
    assert_eq!(again.id, first.id);
    wait(&app, first.id).await;
    let after = app
        .ingestion
        .start_ingestion(app.graph_id, source, "key-1", customer_mapping())
        .await
        .unwrap();
    assert_eq!(after.id, first.id, "also after completion");
    assert_eq!(
        count(&app, app.graph_id, "Customer").await,
        3,
        "ingested once"
    );

    let mut different = customer_mapping();
    different.entities[0].properties.pop();
    assert!(matches!(
        app.ingestion
            .start_ingestion(app.graph_id, source, "key-1", different)
            .await,
        Err(ApplicationError::Conflict(_))
    ));
    let second = app
        .ingestion
        .start_ingestion(app.graph_id, source, "key-2", customer_mapping())
        .await
        .unwrap();
    assert_ne!(second.id, first.id);
    let summary = wait(&app, second.id).await.result.unwrap();
    assert_eq!((summary.entities_created, summary.entities_matched), (0, 3));
}

#[tokio::test]
async fn ingestion_requests_are_validated_before_a_job_exists() {
    let app = app().await;
    app.ontology
        .create_version(app.graph_id, &fraud_ontology())
        .await
        .unwrap();
    let pending = upload(&app, "customers.csv", CUSTOMERS).await.unwrap().id();
    assert!(matches!(
        app.ingestion
            .start_ingestion(app.graph_id, pending, "k", customer_mapping())
            .await,
        Err(ApplicationError::Conflict(_))
    ));
    let ready = analysed(&app, "customers.csv", CUSTOMERS).await;
    let mut invalid = customer_mapping();
    invalid.entities[0].entity_type = "Merchant".into();
    match app
        .ingestion
        .start_ingestion(app.graph_id, ready, "k", invalid)
        .await
    {
        Err(ApplicationError::Unprocessable { code, message }) => {
            assert_eq!(code, "INVALID_MAPPING");
            assert!(message.contains("Merchant"));
        }
        other => panic!("{other:?}"),
    }
    for key in ["", "has space", &"k".repeat(256)] {
        assert!(matches!(
            app.ingestion
                .start_ingestion(app.graph_id, ready, key, customer_mapping())
                .await,
            Err(ApplicationError::InvalidRequest(_))
        ));
    }
    assert!(
        app.metadata
            .find_job_by_idempotency_key(app.graph_id, "k")
            .await
            .unwrap()
            .is_none(),
        "no job was created"
    );
}

#[tokio::test]
async fn bad_rows_fail_the_job_without_writes() {
    let app = app().await;
    app.ontology
        .create_version(app.graph_id, &fraud_ontology())
        .await
        .unwrap();
    // `age` is profiled as a string column, but the ontology says integer.
    let source = analysed(
        &app,
        "customers.csv",
        "customer_id,name,age\nC1,Ann,34\nC2,Bob,old\n",
    )
    .await;
    let job = app
        .ingestion
        .start_ingestion(app.graph_id, source, "k", customer_mapping())
        .await
        .unwrap();
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Failed);
    let error = done.error.unwrap();
    assert_eq!(error.code, "TYPE_CONVERSION");
    assert!(
        error.message.contains("record 2") && !error.message.contains("old"),
        "{}",
        error.message
    );
    assert_eq!(count(&app, app.graph_id, "Customer").await, 0);
    assert_eq!(
        app.graphs.get_graph(app.graph_id).await.unwrap().status(),
        GraphStatus::Draft
    );
}

#[tokio::test]
async fn jobs_use_the_pinned_ontology_version() {
    let gate = Arc::new(Semaphore::new(0));
    let app = app_with(IngestionConfig::default(), Some(gate.clone())).await;
    app.ontology
        .create_version(app.graph_id, &fraud_ontology())
        .await
        .unwrap();
    let source = upload(&app, "customers.csv", CUSTOMERS).await.unwrap().id();
    let analysis = app
        .ingestion
        .start_analysis(app.graph_id, source)
        .await
        .unwrap();
    gate.add_permits(1);
    wait(&app, analysis.id).await;

    let job = app
        .ingestion
        .start_ingestion(app.graph_id, source, "k", customer_mapping())
        .await
        .unwrap();
    // While the job waits for its file, a new version drops `Customer`.
    app.ontology
        .create_version(app.graph_id, &OntologyDefinition::default())
        .await
        .unwrap();
    gate.add_permits(2); // preflight and write passes
    let done = wait(&app, job.id).await;
    assert_eq!(done.status, JobStatus::Completed, "{:?}", done.error);
    assert_eq!(
        done.ontology_version,
        Some(OntologyVersion::new(2).unwrap())
    );
    assert_eq!(count(&app, app.graph_id, "Customer").await, 3);
}

#[tokio::test]
async fn ingestions_of_one_graph_do_not_race() {
    let app = app().await;
    app.ontology
        .create_version(app.graph_id, &fraud_ontology())
        .await
        .unwrap();
    let source = analysed(&app, "customers.csv", CUSTOMERS).await;
    // Two jobs for the same new identities, submitted together with two
    // workers available: the second must match what the first created.
    let first = app
        .ingestion
        .start_ingestion(app.graph_id, source, "race-1", customer_mapping())
        .await
        .unwrap();
    let second = app
        .ingestion
        .start_ingestion(app.graph_id, source, "race-2", customer_mapping())
        .await
        .unwrap();
    let (a, b) = (wait(&app, first.id).await, wait(&app, second.id).await);
    let created = a.result.unwrap().entities_created + b.result.unwrap().entities_created;
    assert_eq!(created, 3, "each customer is created once");
    assert_eq!(count(&app, app.graph_id, "Customer").await, 3);
}

#[tokio::test]
async fn restart_recovery_fails_incomplete_jobs() {
    let app = app().await;
    let source = upload(&app, "customers.csv", CUSTOMERS).await.unwrap().id();
    let now = chrono::Utc::now();
    let queued = IngestionJob::analysis(app.graph_id, source, now);
    let running = IngestionJob::analysis(app.graph_id, source, now);
    let finished = IngestionJob::analysis(app.graph_id, source, now);
    for job in [&queued, &running, &finished] {
        app.metadata.create_job(job).await.unwrap();
    }
    app.metadata.start_job(running.id).await.unwrap();
    app.metadata.start_job(finished.id).await.unwrap();
    app.metadata.complete_job(finished.id, None).await.unwrap();

    assert_eq!(app.ingestion.recover_incomplete_jobs().await.unwrap(), 2);
    for id in [queued.id, running.id] {
        let job = app.ingestion.get_job(id).await.unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.unwrap().code, PROCESS_RESTARTED);
    }
    assert_eq!(
        app.ingestion.get_job(finished.id).await.unwrap().status,
        JobStatus::Completed
    );
    assert_eq!(app.ingestion.recover_incomplete_jobs().await.unwrap(), 0);
}

#[tokio::test]
async fn unknown_jobs_are_not_found() {
    let app = app().await;
    assert!(matches!(
        app.ingestion.get_job(IngestionJobId::new()).await,
        Err(ApplicationError::JobNotFound(_))
    ));
}

#[tokio::test]
async fn invalid_configuration_is_rejected() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let ports = || IngestionPorts {
        metadata: metadata.clone(),
        graph: Arc::new(MemoryGraphStore::new()),
        sources: Arc::new(MemorySourceStore::new()),
    };
    for config in [
        IngestionConfig {
            batch_size: 0,
            ..IngestionConfig::default()
        },
        IngestionConfig {
            job_concurrency: 0,
            ..IngestionConfig::default()
        },
    ] {
        assert!(IngestionService::start(ports(), WorkspaceId::new(), config).is_err());
    }
}
