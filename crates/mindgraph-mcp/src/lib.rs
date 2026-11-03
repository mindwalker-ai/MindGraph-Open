//! MCP server for MindGraph Open (ADR-011).
//!
//! Exposes small, composable, read-only graph tools (`search_entities`,
//! `get_entity`, `get_neighbors`, `find_path`, `get_subgraph`,
//! `query_graph`) to agents such as Claude, Codex and Cursor. It uses the
//! same `mindgraph-core` application layer as the HTTP API and never
//! duplicates graph logic. The stdio transport comes first.
//!
//! Status: skeleton. Implemented in M8 with `rmcp`.
