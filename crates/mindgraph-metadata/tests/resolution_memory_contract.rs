//! Runs the shared `EntityResolutionStore` contract against
//! `MemoryEntityResolutionStore`, the reference implementation, so both
//! backends are held to one contract. Offline.

#[macro_use]
mod resolution_contract;

use std::sync::Arc;

use mindgraph_core::{MemoryEntityResolutionStore, MemoryMetadataStore};
use resolution_contract::Stores;

async fn memory_stores() -> Stores {
    Stores {
        metadata: Arc::new(MemoryMetadataStore::new()),
        resolution: Arc::new(MemoryEntityResolutionStore::new()),
    }
}

resolution_store_contract!([] memory_stores);
