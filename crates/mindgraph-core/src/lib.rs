//! Application layer of MindGraph Open (ADR-002).
//!
//! `mindgraph-core` runs use cases (create a graph, ingest a dataset, find a
//! path) on top of `mindgraph-domain`. The HTTP API, the MCP server and the
//! CLI all call into this crate instead of talking to infrastructure
//! directly (ADR-004, ADR-011):
//!
//! ```text
//! mindgraph-api / mindgraph-mcp / mindgraph-cli
//!                  │
//!                  ▼
//!            mindgraph-core
//!                  │
//!                  ▼
//!           mindgraph-domain
//! ```
//!
//! Status: skeleton. Use cases arrive with M2 (GraphStore) and M3 (project
//! and metadata APIs).
