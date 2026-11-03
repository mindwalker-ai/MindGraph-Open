//! Application layer of MindGraph Open (ADR-002).
//!
//! `mindgraph-core` runs use cases on top of `mindgraph-domain`. It reaches
//! infrastructure only through ports, so the HTTP API, the MCP server and the
//! CLI share one implementation of every use case:
//!
//! ```text
//! mindgraph-api / mindgraph-mcp / mindgraph-cli
//!                  │
//!                  ▼
//!            mindgraph-core ── MetadataStore (port) ◄── mindgraph-metadata (PostgreSQL)
//!                  │
//!                  ▼
//!           mindgraph-domain
//! ```
//!
//! This crate has no dependency on Axum, SQLx or PostgreSQL types.

#![warn(missing_docs)]

mod graph_projects;
mod metadata;

pub use graph_projects::{
    ApplicationError, CreateGraphProject, DEFAULT_LIST_LIMIT, GraphProjectService, MAX_LIST_LIMIT,
};
pub use metadata::{LOCAL_WORKSPACE_NAME, MemoryMetadataStore, MetadataStore, MetadataStoreError};
