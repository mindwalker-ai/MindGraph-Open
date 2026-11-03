//! Performance sanity checks (not an SLA, not part of CI). They exist to
//! catch pathological behaviour such as one full graph scan per row.
//!
//! ```bash
//! cargo test --release -p mindgraph-api --test ingestion_perf -- --ignored --nocapture
//! # The HugeGraph variant also needs `make hugegraph-test-up`.
//! ```
//!
//! Each run generates `customers.csv` (10,000 rows) and `accounts.csv`
//! (10,000 rows, each owned by an existing customer), analyses and ingests
//! both, and prints throughput and entity counts.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mindgraph_core::{
    CreateGraphProject, EntityTypeInput, GraphProjectService, IngestionConfig, IngestionJob,
    IngestionPorts, IngestionService, JobStatus, MemoryMetadataStore, MetadataStore,
    OntologyDefinition, OntologyService, PropertyInput, RelationshipTypeInput,
};
use mindgraph_domain::{DataSourceId, GraphId, IngestionJobId, PropertyType, TypeName};
use mindgraph_graph::{
    EntitySearch, GraphStore, HugeGraphConfig, HugeGraphStore, MemoryGraphStore,
};
use mindgraph_ingestion::{
    EntityMapping, EntityMode, FieldMapping, IngestionMapping, MemorySourceStore,
    RelationshipMapping,
};

fn customers_csv(rows: usize) -> String {
    let mut csv = String::from("customer_id,name,age,city\n");
    for i in 0..rows {
        writeln!(csv, "C{i:06},Customer {i},{},City {}", 18 + i % 60, i % 40).unwrap();
    }
    csv
}

/// Each account belongs to a distinct customer: a worst case for identity
/// lookups (the job cache cannot help).
fn accounts_csv(rows: usize, customers: usize) -> String {
    let mut csv = String::from("account_id,customer_id,balance\n");
    for i in 0..rows {
        writeln!(csv, "A{i:06},C{:06},{}.25", i % customers, i * 3).unwrap();
    }
    csv
}

fn property(name: &str, property_type: PropertyType, required: bool) -> PropertyInput {
    PropertyInput {
        name: name.into(),
        property_type,
        required,
    }
}

fn field(name: &str) -> FieldMapping {
    FieldMapping {
        source_field: name.into(),
        property: name.into(),
    }
}

struct Bench {
    ingestion: IngestionService,
    graph: Arc<dyn GraphStore>,
    graph_id: GraphId,
}

async fn bench(graph: Arc<dyn GraphStore>) -> Bench {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let graphs = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let workspace = graphs.workspace_id();
    let graph_id = graphs
        .create_graph(CreateGraphProject {
            name: "Perf".into(),
            description: None,
        })
        .await
        .unwrap()
        .id();
    OntologyService::new(metadata.clone(), workspace)
        .create_version(
            graph_id,
            &OntologyDefinition {
                entity_types: vec![
                    EntityTypeInput {
                        name: "Customer".into(),
                        display_name: None,
                        description: None,
                        properties: vec![
                            property("customer_id", PropertyType::String, true),
                            property("name", PropertyType::String, true),
                            property("age", PropertyType::Integer, false),
                            property("city", PropertyType::String, false),
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
            },
        )
        .await
        .unwrap();
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata: metadata.clone(),
            graph: graph.clone(),
            sources: Arc::new(MemorySourceStore::new()),
        },
        workspace,
        IngestionConfig::default(),
    )
    .unwrap();
    Bench {
        ingestion,
        graph,
        graph_id,
    }
}

async fn wait(bench: &Bench, id: IngestionJobId) -> (IngestionJob, Duration) {
    let started = Instant::now();
    loop {
        let job = bench.ingestion.get_job(id).await.unwrap();
        if job.status.is_terminal() {
            assert_eq!(job.status, JobStatus::Completed, "{:?}", job.error);
            return (job, started.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[allow(clippy::cast_precision_loss)] // Rates for a printout.
fn rate(rows: usize, elapsed: Duration) -> f64 {
    rows as f64 / elapsed.as_secs_f64()
}

async fn load(bench: &Bench, label: &str, csv: &str, rows: usize, mapping: IngestionMapping) {
    let mut upload = bench
        .ingestion
        .begin_upload(bench.graph_id, &format!("{label}.csv"))
        .await
        .unwrap();
    for chunk in csv.as_bytes().chunks(64 * 1024) {
        upload.write_chunk(chunk).await.unwrap();
    }
    let source: DataSourceId = upload.finish().await.unwrap().id();
    let analysis = bench
        .ingestion
        .start_analysis(bench.graph_id, source)
        .await
        .unwrap();
    let (_, analysis_time) = wait(bench, analysis.id).await;
    let job = bench
        .ingestion
        .start_ingestion(bench.graph_id, source, label, mapping)
        .await
        .unwrap();
    let (job, ingestion_time) = wait(bench, job.id).await;
    let summary = *job.ingestion_summary().unwrap();
    assert_eq!(summary.records_processed, rows as u64);
    eprintln!(
        "{label:>9}: {rows} rows ({} KiB) | analysis {analysis_time:.2?} ({:.0} rows/s) | \
         ingestion {ingestion_time:.2?} ({:.0} rows/s) | created {} matched {} relationships {}",
        csv.len() / 1024,
        rate(rows, analysis_time),
        rate(rows, ingestion_time),
        summary.entities_created,
        summary.entities_matched,
        summary.relationships_created,
    );
}

async fn run(bench: &Bench, customers: usize, accounts: usize) {
    load(
        bench,
        "customers",
        &customers_csv(customers),
        customers,
        IngestionMapping {
            entities: vec![EntityMapping {
                alias: "customer".into(),
                entity_type: "Customer".into(),
                mode: EntityMode::CreateOrMatch,
                identity: vec![field("customer_id")],
                properties: vec![
                    field("customer_id"),
                    field("name"),
                    field("age"),
                    field("city"),
                ],
            }],
            relationships: vec![],
        },
    )
    .await;
    load(
        bench,
        "accounts",
        &accounts_csv(accounts, customers),
        accounts,
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
        },
    )
    .await;
    let count = |entity_type: &'static str| async move {
        let search = EntitySearch::new()
            .with_entity_type(TypeName::new(entity_type).unwrap())
            .with_limit(1)
            .unwrap();
        // Existence only: listing every entity is capped at 1,000 per call.
        !bench
            .graph
            .search_entities(bench.graph_id, &search)
            .await
            .unwrap()
            .is_empty()
    };
    assert!(count("Customer").await && count("Account").await);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "performance sanity check; run with --release --ignored --nocapture"]
async fn ten_thousand_rows_in_memory() {
    let bench = bench(Arc::new(MemoryGraphStore::new())).await;
    run(&bench, 10_000, 10_000).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "performance sanity check against HugeGraph 1.7; see module docs"]
async fn ten_thousand_rows_into_hugegraph() {
    let url = std::env::var("MINDGRAPH_HUGEGRAPH_TEST_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:18080".to_owned());
    let store = HugeGraphStore::new(HugeGraphConfig::new(&url).unwrap()).unwrap();
    store.ensure_schema().await.unwrap();
    let store: Arc<dyn GraphStore> = Arc::new(store);
    let bench = bench(store.clone()).await;
    // Lookups are batched (one HugeGraph scan per type per batch),
    // so the full 10,000 accounts are affordable.
    run(&bench, 10_000, 10_000).await;
    store.clear_graph(bench.graph_id).await.unwrap();
}
