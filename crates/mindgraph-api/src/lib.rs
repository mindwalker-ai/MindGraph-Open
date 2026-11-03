//! HTTP API of MindGraph Open (ADR-006).
//!
//! Transport only: handlers translate HTTP into calls on the application
//! layer (`mindgraph-core`) and never touch the database. Routes:
//!
//! ```text
//! GET  /health                          liveness (no database access)
//! GET  /ready                           readiness (checks PostgreSQL)
//! POST /api/v1/graphs                   create a graph project
//! GET  /api/v1/graphs?limit=50          list graph projects, newest first
//! GET  /api/v1/graphs/{graph_id}        read a graph project
//! GET  /api/v1/graphs/{graph_id}/ontology   read its active ontology
//! ```
//!
//! Every response carries an `x-request-id` header. Every error uses the
//! stable envelope `{"error": {"code", "message", "request_id"}}`.

mod config;
mod dto;
mod error;
mod request_context;
mod routes;

pub use config::{Config, ConfigError, DatabaseConfig};
pub use routes::{AppState, router};

/// The service name reported by the health check.
pub const SERVICE_NAME: &str = "mindgraph-core";
