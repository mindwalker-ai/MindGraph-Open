//! The M4 definition of done as one flow through the HTTP router, using the
//! files in `examples/fraud/`. Shared by an offline test (memory graph
//! store) and an opt-in test against HugeGraph 1.7.
//!
//! ```text
//! graph A: ontology → customers.csv → analyze → ingest
//!          accounts.csv → analyze → ingest (Account + Customer lookup + OWNS)
//! graph B: ontology → customers.csv only, and must stay untouched by A
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use mindgraph_api::{AppState, router};
use mindgraph_core::{
    GraphProjectService, IngestionConfig, IngestionPorts, IngestionService, MemoryMetadataStore,
    MetadataStore, OntologyService,
};
use mindgraph_domain::{Entity, GraphId, TypeName};
use mindgraph_graph::{Direction, EntitySearch, GraphStore, NeighborQuery};
use mindgraph_ingestion::MemorySourceStore;
use serde_json::{Value, json};
use tower::ServiceExt;

const CUSTOMERS: &str = include_str!("../../../../examples/fraud/customers.csv");
const ACCOUNTS: &str = include_str!("../../../../examples/fraud/accounts.csv");
const ONTOLOGY: &str = include_str!("../../../../examples/fraud/ontology.json");
const CUSTOMERS_MAPPING: &str = include_str!("../../../../examples/fraud/customers.mapping.json");
const ACCOUNTS_MAPPING: &str = include_str!("../../../../examples/fraud/accounts.mapping.json");
const BOUNDARY: &str = "fraud-example-boundary";

/// The router with in-memory metadata and source storage over `graph`.
pub async fn app(graph: Arc<dyn GraphStore>) -> Router {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let graph_projects = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let workspace_id = graph_projects.workspace_id();
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata: metadata.clone(),
            graph,
            sources: Arc::new(MemorySourceStore::new()),
        },
        workspace_id,
        // Small batches, so the example exercises several write batches.
        IngestionConfig {
            batch_size: 2,
            ..IngestionConfig::default()
        },
    )
    .unwrap();
    router(AppState::new(
        graph_projects,
        OntologyService::new(metadata, workspace_id),
        ingestion,
    ))
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn post(app: &Router, uri: &str, body: &str, key: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::post(uri).header(header::CONTENT_TYPE, "application/json");
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    call(app, request.body(Body::from(body.to_owned())).unwrap()).await
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    call(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

async fn wait(app: &Router, job_id: &str) -> Value {
    for _ in 0..1_200 {
        let (status, job) = get(app, &format!("/api/v1/jobs/{job_id}")).await;
        assert_eq!(status, StatusCode::OK, "{job}");
        if matches!(job["status"].as_str(), Some("completed" | "failed")) {
            assert_eq!(job["status"], "completed", "{job}");
            return job;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("job {job_id} did not finish");
}

/// Upload → analyze → ingest one file; returns the ingestion job.
async fn load(app: &Router, graph: &str, filename: &str, content: &str, mapping: &str) -> Value {
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
         Content-Type: text/csv\r\n\r\n{content}\r\n--{BOUNDARY}--\r\n"
    );
    let request = Request::post(format!("/api/v1/graphs/{graph}/data-sources"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap();
    let (status, source) = call(app, request).await;
    assert_eq!(status, StatusCode::CREATED, "{source}");
    let source_id = source["id"].as_str().unwrap();
    let base = format!("/api/v1/graphs/{graph}/data-sources/{source_id}");

    let (status, accepted) = post(app, &format!("{base}/analyze"), "{}", None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    wait(app, accepted["job_id"].as_str().unwrap()).await;
    let (_, analysed) = get(app, &base).await;
    assert_eq!(analysed["status"], "ready", "{analysed}");

    let key = format!("{graph}-{filename}");
    let (status, job) = post(app, &format!("{base}/ingestions"), mapping, Some(&key)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    wait(app, job["id"].as_str().unwrap()).await
}

async fn create_graph(app: &Router, name: &str) -> String {
    let body = json!({ "name": name }).to_string();
    let (status, graph) = post(app, "/api/v1/graphs", &body, None).await;
    assert_eq!(status, StatusCode::CREATED, "{graph}");
    let id = graph["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v1/graphs/{id}/ontology");
    let (status, ontology) = post(app, &uri, ONTOLOGY, None).await;
    assert_eq!(status, StatusCode::CREATED, "{ontology}");
    id
}

async fn of_type(store: &dyn GraphStore, graph_id: GraphId, entity_type: &str) -> Vec<Entity> {
    let search = EntitySearch::new()
        .with_entity_type(TypeName::new(entity_type).unwrap())
        .with_limit(1_000)
        .unwrap();
    store.search_entities(graph_id, &search).await.unwrap()
}

fn text(entity: &Entity, key: &str) -> String {
    entity
        .properties()
        .get(key)
        .unwrap()
        .to_json()
        .as_str()
        .unwrap()
        .to_owned()
}

/// Runs the flow and checks the graph contents. Returns the two graph ids so
/// callers can clean up.
pub async fn run(app: &Router, store: &dyn GraphStore) -> [GraphId; 2] {
    let graph_a = create_graph(app, "Fraud A").await;
    let graph_b = create_graph(app, "Fraud B").await;
    let b_job = load(app, &graph_b, "customers.csv", CUSTOMERS, CUSTOMERS_MAPPING).await;
    assert_eq!(b_job["result"]["entities_created"], 5);

    let customers = load(app, &graph_a, "customers.csv", CUSTOMERS, CUSTOMERS_MAPPING).await;
    assert_eq!(
        customers["result"],
        json!({"records_processed": 5, "entities_created": 5, "entities_matched": 0, "relationships_created": 0})
    );
    let accounts = load(app, &graph_a, "accounts.csv", ACCOUNTS, ACCOUNTS_MAPPING).await;
    assert_eq!(
        accounts["result"],
        json!({"records_processed": 7, "entities_created": 7, "entities_matched": 5, "relationships_created": 7})
    );
    let (_, project) = get(app, &format!("/api/v1/graphs/{graph_a}")).await;
    assert_eq!(project["status"], "active");

    let a: GraphId = graph_a.parse().unwrap();
    let b: GraphId = graph_b.parse().unwrap();
    let customers = of_type(store, a, "Customer").await;
    assert_eq!(customers.len(), 5);
    assert_eq!(of_type(store, a, "Account").await.len(), 7);
    let alice = customers
        .iter()
        .find(|c| text(c, "customer_id") == "C001")
        .unwrap();
    assert_eq!(text(alice, "name"), "Alice Tan");
    let citra = customers
        .iter()
        .find(|c| text(c, "customer_id") == "C003")
        .unwrap();
    assert!(
        !citra.properties().contains_key("age"),
        "blank optional values are omitted"
    );

    let owned = store
        .get_neighbors(a, alice.id(), &NeighborQuery::new(Direction::Outgoing))
        .await
        .unwrap();
    let mut owned_ids: Vec<String> = owned
        .entities()
        .iter()
        .filter(|entity| entity.id() != alice.id())
        .map(|account| text(account, "account_id"))
        .collect();
    owned_ids.sort();
    assert_eq!(owned_ids, ["A1001", "A1002"]);
    assert!(
        owned
            .relationships()
            .iter()
            .all(|r| r.relationship_type().as_str() == "OWNS")
    );

    // Graph B: only its own five customers, no accounts, nothing from A.
    let b_customers = of_type(store, b, "Customer").await;
    assert_eq!(b_customers.len(), 5);
    assert!(of_type(store, b, "Account").await.is_empty());
    assert!(b_customers.iter().all(|c| c.graph_id() == b));
    let ids_a: Vec<_> = customers.iter().map(Entity::id).collect();
    assert!(b_customers.iter().all(|c| !ids_a.contains(&c.id())));
    [a, b]
}
