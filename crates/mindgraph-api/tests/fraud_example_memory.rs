//! The `examples/fraud` M4 flow, offline: in-memory graph, metadata and
//! source storage. The same flow runs against HugeGraph in
//! `hugegraph_ingestion.rs`.

mod fraud_example;

use std::sync::Arc;

use mindgraph_graph::{GraphStore, MemoryGraphStore};

#[tokio::test]
async fn fraud_example_builds_customers_that_own_accounts() {
    let store: Arc<dyn GraphStore> = Arc::new(MemoryGraphStore::new());
    let app = fraud_example::app(store.clone()).await;
    fraud_example::run(&app, store.as_ref()).await;
}
