//! Dataset ingestion for MindGraph Open (ADR-008).
//!
//! Rust owns the whole ingestion path: CSV and JSON parsing, schema
//! inspection, sampling, primitive type inference, mapping rows to entities
//! and relationships, validation, and batched graph writes. The Python AI
//! service only ever receives schema, statistics and representative samples,
//! never full files.
//!
//! Status: skeleton. Implemented in M4.
