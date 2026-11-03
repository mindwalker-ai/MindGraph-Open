//! Graph-first GraphRAG retrieval for MindGraph Open (ADR-010).
//!
//! The Rust side of the GraphRAG pipeline: entity identification, graph
//! retrieval, subgraph construction and context serialisation. Natural
//! language synthesis is delegated to the Python AI service. Answers always
//! carry an evidence graph, and AI-generated relationships are never
//! persisted from an answer.
//!
//! Status: skeleton. Implemented in M7.
