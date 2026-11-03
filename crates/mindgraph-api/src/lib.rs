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
//! POST /api/v1/graphs/{graph_id}/ontology   create + activate the next version
//! POST /api/v1/graphs/{graph_id}/data-sources            upload (multipart `file`)
//! GET  /api/v1/graphs/{graph_id}/data-sources/{id}       read, with profile
//! POST /api/v1/graphs/{graph_id}/data-sources/{id}/analyze      202 + job
//! POST /api/v1/graphs/{graph_id}/data-sources/{id}/ingestions   202 + job (Idempotency-Key)
//! POST /api/v1/graphs/{graph_id}/data-sources/{id}/graph-builder/suggestions
//!                                           202 + AI suggestion job (Idempotency-Key)
//! GET  /api/v1/graph-builder/status         is the AI Graph Builder available?
//! GET  /api/v1/jobs/{job_id}                read a job
//! GET  /api/v1/jobs/{job_id}/events         job progress as Server-Sent Events
//! ```
//!
//! Every response carries an `x-request-id` header. Every error uses the
//! stable envelope `{"error": {"code", "message", "request_id"}}`.

mod config;
mod datasets;
mod dto;
mod error;
mod request_context;
mod routes;

pub use config::{AiSettings, Config, ConfigError, DatabaseConfig, HugeGraphSettings};
pub use routes::{AppState, router};

/// The service name reported by the health check.
pub const SERVICE_NAME: &str = "mindgraph-core";
