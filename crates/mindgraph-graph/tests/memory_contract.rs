//! Runs the shared `GraphStore` contract against `MemoryGraphStore`.

#[macro_use]
mod contract;

use std::sync::Arc;

use mindgraph_graph::{GraphStore, MemoryGraphStore};

async fn memory_store() -> Arc<dyn GraphStore> {
    Arc::new(MemoryGraphStore::new())
}

graph_store_contract!([] memory_store);
