//! PostgreSQL implementation of the MindGraph metadata store (ADR-005).
//!
//! Implements [`mindgraph_core::MetadataStore`] with SQLx and explicit SQL.
//! PostgreSQL holds control-plane metadata only (workspaces, graph projects,
//! ontology versions); graph data lives in the `GraphStore`.
//!
//! Queries are built at runtime, never checked against a live database at
//! compile time, so building needs no database. The SQL migrations in
//! `/migrations` are embedded into the binary and applied at startup by
//! [`PostgresMetadataStore::migrate`].

#![warn(missing_docs)]

mod codec;
mod store;

pub use store::{PostgresMetadataStore, PostgresOptions};

/// The embedded schema migrations from the repository's `migrations/`
/// directory.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
