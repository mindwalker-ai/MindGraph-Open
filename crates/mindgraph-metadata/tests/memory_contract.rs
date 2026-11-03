//! Runs the shared `MetadataStore` contract against `MemoryMetadataStore`,
//! the reference implementation, so both backends are held to one contract.

#[macro_use]
mod contract;

use std::sync::Arc;

use mindgraph_core::{MemoryMetadataStore, MetadataStore};

async fn memory_store() -> Arc<dyn MetadataStore> {
    Arc::new(MemoryMetadataStore::new())
}

metadata_store_contract!([] memory_store);
