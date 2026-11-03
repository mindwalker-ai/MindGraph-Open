//! Graph storage for MindGraph Open (ADR-004).
//!
//! This crate will define the `GraphStore` trait and its adapters:
//!
//! - `MemoryGraphStore`, for unit tests, local development and benchmarks
//! - `HugeGraphStore`, the Apache HugeGraph adapter used in deployments
//!
//! Application code never calls HugeGraph directly. It always goes through
//! `GraphStore`, so the backend can later be replaced (for example by a
//! native MindGraph DB) without changing application logic.
//!
//! Status: skeleton. Implemented in M2.
