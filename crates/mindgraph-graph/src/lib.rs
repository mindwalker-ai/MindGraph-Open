//! Graph storage for MindGraph Open (ADR-004).
//!
//! Application code talks to graph storage only through the [`GraphStore`]
//! trait. Two implementations ship with v0.1:
//!
//! - [`MemoryGraphStore`]: the in-memory reference implementation of the
//!   contract, for unit tests, local development and benchmarks.
//! - [`HugeGraphStore`]: the Apache HugeGraph 1.7 adapter used in deployments.
//!
//! Both implement the same behavioural contract (documented on
//! [`GraphStore`] and exercised by one shared test suite), so application code
//! never needs to know which backend is active:
//!
//! ```
//! use std::sync::Arc;
//!
//! use mindgraph_domain::{Entity, EntityId, GraphId, Properties, TypeName};
//! use mindgraph_graph::{GraphStore, MemoryGraphStore, NeighborQuery};
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store: Arc<dyn GraphStore> = Arc::new(MemoryGraphStore::new());
//!
//! let graph_id = GraphId::new();
//! let customer = Entity::new(EntityId::new(), graph_id, TypeName::new("Customer")?, Properties::new());
//! let customer_id = customer.id();
//! store.create_entity(customer).await?;
//!
//! let neighborhood = store
//!     .get_neighbors(graph_id, customer_id, &NeighborQuery::default())
//!     .await?;
//! assert_eq!(neighborhood.entities().len(), 1);
//! # Ok(())
//! # }
//! ```
//!
//! HugeGraph-specific concepts (vertex labels, REST payloads, the physical
//! property encoding) stay private to this crate.

#![warn(missing_docs)]

mod error;
mod hugegraph;
mod memory;
mod query;
mod store;
mod validate;

pub use error::GraphStoreError;
pub use hugegraph::{HugeGraphConfig, HugeGraphStore};
pub use memory::MemoryGraphStore;
pub use query::{
    Direction, EntitySearch, GraphPath, MAX_SUBGRAPH_ENTITIES, NeighborQuery, PathQuery, Subgraph,
};
pub use store::GraphStore;
