//! The M9 entity resolution flow against a live HugeGraph 1.7.0: metadata
//! and decisions in memory, graph data in `HugeGraphStore`. Proves that
//! candidate detection, review and approval work on the real backend and
//! that approval leaves every original entity, property and relationship in
//! HugeGraph unchanged.
//!
//! Ignored by default, so `cargo test --workspace` stays offline. To run:
//!
//! ```bash
//! make hugegraph-test-up      # HugeGraph 1.7 on 127.0.0.1:18080
//! make test-hugegraph         # also runs this suite
//! ```
//!
//! `MINDGRAPH_HUGEGRAPH_TEST_URL` (default `http://127.0.0.1:18080`) selects
//! the server. The test writes to two fresh random graph ids and clears them
//! afterwards; it never clears the HugeGraph database itself. No model is
//! needed: entity resolution is deterministic.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

mod resolution_flow;

use std::sync::Arc;
use std::time::Duration;

use mindgraph_graph::{GraphStore, GraphStoreError, HugeGraphConfig, HugeGraphStore};

const READY_TIMEOUT: Duration = Duration::from_secs(180);

async fn ready_store() -> HugeGraphStore {
    let url = std::env::var("MINDGRAPH_HUGEGRAPH_TEST_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:18080".to_owned());
    let store = HugeGraphStore::new(HugeGraphConfig::new(&url).unwrap()).unwrap();
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
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
#[ignore = "requires HugeGraph 1.7; see module docs"]
async fn entity_resolution_leaves_hugegraph_untouched() {
    let store: Arc<dyn GraphStore> = Arc::new(ready_store().await);
    let app = resolution_flow::app(store.clone()).await;
    let graphs = resolution_flow::run(&app, store.as_ref()).await;
    for graph_id in graphs {
        store.clear_graph(graph_id).await.unwrap();
    }
}
