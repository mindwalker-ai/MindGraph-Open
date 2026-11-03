//! Live contract tests against a real HugeGraph 1.7.0 server.
//!
//! Ignored by default, so `cargo test --workspace` stays offline. To run:
//!
//! ```bash
//! # Docker or Podman; HugeGraph is published on localhost only.
//! docker run -d --name mindgraph-hugegraph-test -p 127.0.0.1:18080:8080 hugegraph/hugegraph:1.7.0
//! cargo test -p mindgraph-graph --test hugegraph_integration -- --ignored
//! ```
//!
//! Environment variables (all optional):
//!
//! - `MINDGRAPH_HUGEGRAPH_TEST_URL` (default `http://127.0.0.1:18080`)
//! - `MINDGRAPH_HUGEGRAPH_TEST_GRAPHSPACE` (default `DEFAULT`)
//! - `MINDGRAPH_HUGEGRAPH_TEST_GRAPH` (default `hugegraph`)
//!
//! Every test writes to fresh random graph ids and clears them afterwards,
//! so it is safe to run against a shared development server. It never
//! clears the HugeGraph database itself.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[macro_use]
mod contract;

use std::env;
use std::sync::Arc;
use std::time::Duration;

use mindgraph_graph::{GraphStore, GraphStoreError, HugeGraphConfig, HugeGraphStore};

const READY_TIMEOUT: Duration = Duration::from_secs(180);

fn config() -> HugeGraphConfig {
    let url = env::var("MINDGRAPH_HUGEGRAPH_TEST_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:18080".to_owned());
    let mut config = HugeGraphConfig::new(&url).expect("valid MINDGRAPH_HUGEGRAPH_TEST_URL");
    if let Ok(graphspace) = env::var("MINDGRAPH_HUGEGRAPH_TEST_GRAPHSPACE") {
        config = config.with_graphspace(&graphspace).unwrap();
    }
    if let Ok(graph) = env::var("MINDGRAPH_HUGEGRAPH_TEST_GRAPH") {
        config = config.with_graph_name(&graph).unwrap();
    }
    config
}

/// Builds a store and waits until HugeGraph accepts the schema bootstrap.
async fn ready_store() -> HugeGraphStore {
    let store = HugeGraphStore::new(config()).unwrap();
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        match store.ensure_schema().await {
            Ok(()) => return store,
            Err(GraphStoreError::BackendUnavailable { .. })
                if tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(err) => panic!(
                "HugeGraph at {} is not usable: {err}",
                store.config().base_url()
            ),
        }
    }
}

async fn hugegraph_store() -> Arc<dyn GraphStore> {
    Arc::new(ready_store().await)
}

graph_store_contract!([ignore = "requires HugeGraph 1.7; see module docs"] hugegraph_store);

#[tokio::test]
#[ignore = "requires HugeGraph 1.7; see module docs"]
async fn schema_bootstrap_is_idempotent() {
    let store = ready_store().await;
    store.ensure_schema().await.unwrap();
    store.ensure_schema().await.unwrap();
}

#[tokio::test]
#[ignore = "requires HugeGraph 1.7; see module docs"]
async fn batches_larger_than_one_hugegraph_request_are_written() {
    use mindgraph_domain::{Entity, EntityId, GraphId, Properties, TypeName};
    use mindgraph_graph::EntitySearch;

    let store = ready_store().await;
    let graph_id = GraphId::new();
    // 1,200 entities: three HugeGraph batch requests of at most 500.
    let entities: Vec<Entity> = (0..1_200)
        .map(|_| {
            Entity::new(
                EntityId::new(),
                graph_id,
                TypeName::new("Node").unwrap(),
                Properties::new(),
            )
        })
        .collect();
    store.create_entities(entities.clone()).await.unwrap();
    let found = store
        .search_entities(graph_id, &EntitySearch::new().with_limit(1_000).unwrap())
        .await
        .unwrap();
    assert_eq!(found.len(), 1_000, "capped by the search limit");
    assert_eq!(
        found.first().map(Entity::id),
        entities.first().map(Entity::id)
    );
    store.clear_graph(graph_id).await.unwrap();
    assert!(
        store
            .search_entities(graph_id, &EntitySearch::new())
            .await
            .unwrap()
            .is_empty()
    );
}
