//! An agent's fraud investigation over MCP, end to end:
//!
//! ```text
//! rmcp client ──stdio──► mindgraph-mcp ──HTTP──► MindGraph Core (real router)
//!                                                └─ GraphStore (memory or HugeGraph)
//! ```
//!
//! The fraud example is loaded through Core's REST API as a user would
//! (graph A: customers, accounts, devices; graph B: customers only). The
//! agent then calls get_graph_info → get_graph_schema → search_entities →
//! get_neighbors → find_path → get_subgraph (and ask_graph), and every
//! result is checked against the CSV files and Core's own answers. A spy on
//! the GraphStore proves that no MCP call writes anything.
//!
//! - `an_agent_finds_the_shared_device_over_mcp`: offline, in-memory store.
//! - `..._on_hugegraph` (ignored): the same against HugeGraph 1.7
//!   (`make hugegraph-test-up`, `MINDGRAPH_HUGEGRAPH_TEST_URL`).
//! - `..._on_a_running_core` (ignored): the same against a MindGraph Core
//!   you started yourself (`docker compose up` or `make dev-core`), given as
//!   `MINDGRAPH_MCP_LIVE_CORE_URL`. It creates two "MCP live" graphs there.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

mod support;

use std::sync::Arc;
use std::time::Duration;

use mindgraph_graph::{
    GraphStore, GraphStoreError, HugeGraphConfig, HugeGraphStore, MemoryGraphStore,
};
use serde_json::{Value, json};

use support::{
    Fraud, WriteSpy, call, code, connect, core_find, core_get, load_fraud, ok, spawn_core,
};

/// What to expect from `ask_graph`.
#[derive(Clone, Copy)]
enum Ask {
    /// Core runs GraphRAG with the deterministic test model.
    FakeModel,
    /// A real Core whose model may or may not be configured.
    Whatever,
}

fn values(entities: &Value, key: &str) -> Vec<String> {
    let mut values: Vec<String> = entities
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entity| entity["properties"][key].as_str().map(str::to_owned))
        .collect();
    values.sort();
    values
}

async fn find(session: &support::Session, entity_type: &str, key: &str, value: &str) -> Value {
    let found = ok(&call(
        session,
        "search_entities",
        json!({"entity_type": entity_type, "property_filters": {key: value}}),
    )
    .await);
    assert_eq!(found["count"], 1, "{found}");
    assert_eq!(found["limit"], 20);
    assert_eq!(found["limit_reached"], false);
    found["entities"][0].clone()
}

/// The deterministic workflow of the M8 fraud demo, plus isolation and
/// read-only checks. Needs no model call except `ask_graph`.
async fn investigate(core: &str, fraud: &Fraud, spy: Option<&WriteSpy>, ask: Ask) {
    let graphs_before = core_get(core, "/api/v1/graphs").await["count"].clone();
    let writes_before = spy.map(WriteSpy::writes);
    let session = connect(
        core,
        &fraud.graph_a,
        &[("MINDGRAPH_MCP_ENABLE_ASK", "true")],
    )
    .await;

    // What graph is this?
    let info = ok(&call(&session, "get_graph_info", json!({})).await);
    assert_eq!(info["id"], fraud.graph_a.as_str());
    assert_eq!(info["status"], "active");
    assert!(info["name"].as_str().unwrap().ends_with(" A"), "{info}");

    // What is in it?
    let schema = ok(&call(&session, "get_graph_schema", json!({})).await);
    let ontology = core_get(core, &format!("/api/v1/graphs/{}/ontology", fraud.graph_a)).await;
    assert_eq!(schema["version"], ontology["version"]);
    let mut types: Vec<&str> = schema["entity_types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    types.sort_unstable();
    assert_eq!(
        types,
        ["Account", "Customer", "Device", "Merchant", "Transaction"]
    );
    let relationship = |name: &str| {
        schema["relationship_types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .cloned()
            .unwrap()
    };
    assert_eq!(relationship("OWNS")["source_entity_type"], "Customer");
    assert_eq!(relationship("OWNS")["target_entity_type"], "Account");
    assert_eq!(relationship("USES")["target_entity_type"], "Device");
    let customer = schema["entity_types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Customer")
        .unwrap();
    assert!(
        customer["properties"]
            .as_array()
            .unwrap()
            .contains(&json!({"name": "customer_id", "type": "string", "required": true})),
        "{customer}"
    );

    // Find Customer C001: the same entity Core finds.
    let alice = find(&session, "Customer", "customer_id", "C001").await;
    assert_eq!(alice["properties"]["name"], "Alice Tan");
    let alice_id = alice["id"].as_str().unwrap().to_owned();
    assert_eq!(
        alice_id,
        core_find(core, &fraud.graph_a, "Customer", "customer_id", "C001").await
    );
    let fetched = ok(&call(&session, "get_entity", json!({"entity_id": alice_id})).await);
    assert_eq!(fetched, alice);

    // What accounts belong to Customer C001? (accounts.csv: A1001, A1002)
    let owned = ok(&call(
        &session,
        "get_neighbors",
        json!({"entity_id": alice_id, "direction": "outgoing", "relationship_type": "OWNS"}),
    )
    .await);
    assert_eq!(values(&owned["entities"], "account_id"), ["A1001", "A1002"]);
    assert!(
        owned["entities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == alice_id.as_str())
    );
    let relationships = owned["relationships"].as_array().unwrap();
    assert_eq!(relationships.len(), 2);
    assert!(
        relationships
            .iter()
            .all(|r| r["relationship_type"] == "OWNS" && r["source_id"] == alice_id.as_str())
    );
    assert_eq!(owned["limit"], 50);
    assert_eq!(owned["limit_reached"], false);

    // Which customers share Device DEVICE-921? (devices.csv: C001, C002)
    let device = find(&session, "Device", "device_id", "DEVICE-921").await;
    let device_id = device["id"].as_str().unwrap().to_owned();
    let sharing = ok(&call(
        &session,
        "get_neighbors",
        json!({"entity_id": device_id, "direction": "incoming", "relationship_type": "USES"}),
    )
    .await);
    assert_eq!(
        values(&sharing["entities"], "customer_id"),
        ["C001", "C002"]
    );

    // The shortest connection between C001 and C002.
    let budi = find(&session, "Customer", "customer_id", "C002").await;
    let budi_id = budi["id"].as_str().unwrap().to_owned();
    let path = ok(&call(
        &session,
        "find_path",
        json!({"source_id": alice_id, "target_id": budi_id, "max_depth": 4}),
    )
    .await);
    assert_eq!(path["found"], true);
    assert_eq!(path["hops"], 2);
    let keys: Vec<&str> = path["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            e["properties"]["customer_id"]
                .as_str()
                .or_else(|| e["properties"]["device_id"].as_str())
                .unwrap()
        })
        .collect();
    assert_eq!(keys, ["C001", "DEVICE-921", "C002"]);
    let kinds: Vec<&str> = path["relationships"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["relationship_type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["USES", "USES"]);
    assert!(path["note"].is_null());
    // A bounded miss is reported as such, not as "unconnected".
    let short = ok(&call(
        &session,
        "find_path",
        json!({"source_id": alice_id, "target_id": budi_id, "max_depth": 1}),
    )
    .await);
    assert_eq!(short["found"], false);
    assert!(
        short["note"].as_str().unwrap().contains("does not show"),
        "{short}"
    );

    // The subgraph of what we found, plus an entity of graph B.
    let foreign = core_find(core, &fraud.graph_b, "Customer", "customer_id", "C001").await;
    let subgraph = ok(&call(
        &session,
        "get_subgraph",
        json!({"entity_ids": [alice_id, device_id, budi_id, foreign]}),
    )
    .await);
    assert_eq!(subgraph["requested"], 4);
    assert_eq!(subgraph["entities"].as_array().unwrap().len(), 3);
    let relationships = subgraph["relationships"].as_array().unwrap();
    assert_eq!(relationships.len(), 2);
    assert!(
        relationships
            .iter()
            .all(|r| r["relationship_type"] == "USES" && r["target_id"] == device_id.as_str())
    );
    assert_eq!(subgraph["missing_ids"], json!([foreign]));

    // Graph isolation: graph B's entity does not exist for this server.
    let isolated = call(&session, "get_entity", json!({"entity_id": foreign})).await;
    assert_eq!(code(&isolated), "ENTITY_NOT_FOUND");
    let isolated = call(&session, "get_neighbors", json!({"entity_id": foreign})).await;
    assert_eq!(code(&isolated), "ENTITY_NOT_FOUND");

    // GraphRAG through Core, never a second implementation.
    let answer = call(
        &session,
        "ask_graph",
        json!({"question": "Which customers share device DEVICE-921?"}),
    )
    .await;
    match ask {
        Ask::FakeModel => {
            let answer = ok(&answer);
            assert_eq!(answer["status"], "answered", "{answer}");
            let evidence = &answer["evidence"]["entities"];
            assert_eq!(values(evidence, "customer_id"), ["C001", "C002"]);
            assert_eq!(values(evidence, "device_id"), ["DEVICE-921"]);
            assert!(!answer["citations"].as_array().unwrap().is_empty());
        }
        Ask::Whatever => {
            if answer.is_error == Some(true) {
                let code = code(&answer);
                assert!(
                    ["AI_NOT_CONFIGURED", "AI_UNAVAILABLE", "AI_TIMEOUT"].contains(&code.as_str()),
                    "{code}"
                );
            } else {
                let status = ok(&answer)["status"].as_str().unwrap().to_owned();
                assert!(
                    ["answered", "partial", "insufficient_evidence"].contains(&status.as_str())
                );
            }
        }
    }

    // Read-only: nothing reached the GraphStore's write methods, no graph
    // project or ontology version appeared, and graph B is intact.
    session.cancel().await.unwrap();
    if let (Some(spy), Some(before)) = (spy, writes_before) {
        assert_eq!(spy.writes(), before, "MCP tools never write to the graph");
    }
    assert_eq!(
        core_get(core, "/api/v1/graphs").await["count"],
        graphs_before
    );
    assert_eq!(
        core_get(core, &format!("/api/v1/graphs/{}/ontology", fraud.graph_a)).await["version"],
        ontology["version"]
    );
    let customers_b = reqwest::Client::new()
        .post(format!(
            "{core}/api/v1/graphs/{}/entities/search",
            fraud.graph_b
        ))
        .json(&json!({"entity_type": "Customer"}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(customers_b["count"], 5);
}

#[tokio::test]
async fn an_agent_finds_the_shared_device_over_mcp() {
    let spy = Arc::new(WriteSpy::new(Arc::new(MemoryGraphStore::new())));
    let core = spawn_core(spy.clone()).await;
    let fraud = load_fraud(&core.url, "Fraud").await;
    assert!(spy.writes() > 0, "the example was loaded through Core");
    investigate(&core.url, &fraud, Some(&spy), Ask::FakeModel).await;
}

async fn ready_hugegraph() -> HugeGraphStore {
    let url = std::env::var("MINDGRAPH_HUGEGRAPH_TEST_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:18080".to_owned());
    let store = HugeGraphStore::new(HugeGraphConfig::new(&url).unwrap()).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        match store.ensure_schema().await {
            Ok(()) => return store,
            Err(GraphStoreError::BackendUnavailable { .. })
                if tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(err) => panic!("HugeGraph is not usable: {err}"),
        }
    }
}

#[tokio::test]
#[ignore = "requires HugeGraph 1.7; run `make test-hugegraph`"]
async fn an_agent_finds_the_shared_device_over_mcp_on_hugegraph() {
    let hugegraph: Arc<dyn GraphStore> = Arc::new(ready_hugegraph().await);
    let spy = Arc::new(WriteSpy::new(hugegraph.clone()));
    let core = spawn_core(spy.clone()).await;
    let fraud = load_fraud(&core.url, "Fraud").await;
    investigate(&core.url, &fraud, Some(&spy), Ask::FakeModel).await;
    for graph in [&fraud.graph_a, &fraud.graph_b] {
        hugegraph.clear_graph(graph.parse().unwrap()).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires a running MindGraph Core; set MINDGRAPH_MCP_LIVE_CORE_URL"]
async fn an_agent_finds_the_shared_device_over_mcp_on_a_running_core() {
    let core = std::env::var("MINDGRAPH_MCP_LIVE_CORE_URL")
        .expect("MINDGRAPH_MCP_LIVE_CORE_URL, e.g. http://127.0.0.1:8080");
    let core = core.trim_end_matches('/');
    let fraud = load_fraud(core, "MCP live").await;
    eprintln!(
        "graphs created on {core}: A {} B {}",
        fraud.graph_a, fraud.graph_b
    );
    investigate(core, &fraud, None, Ask::Whatever).await;
}
