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
//!                  │        ── GraphStore (port)    ◄── HugeGraphStore (mindgraph-graph)
//!                  │        ── SourceStore (port)   ◄── LocalSourceStore (mindgraph-ingestion)
//!                  │        ── GraphBuilderAi (port) ◄── HttpGraphBuilderAi (mindgraph-ai-client)
//!                  ▼
//!   mindgraph-ingestion (deterministic engine) ─► mindgraph-domain
//! ```
//!
//! This crate has no dependency on Axum, SQLx, PostgreSQL or HTTP client
//! types. It uses Tokio only to run background jobs in-process.

#![warn(missing_docs)]

mod ai;
mod error;
mod executor;
mod explorer;
mod graph_builder;
mod graph_projects;
mod ingestion;
mod jobs;
mod memory;
mod metadata;
mod ontology;

pub use ai::{
    AiError, AiMetadata, AiStatus, AiSuggestion, DatasetContext, DatasetMapping,
    DisabledGraphBuilderAi, GraphBuilderAi, GraphBuilderRequest, GraphBuilderSuggestion,
    OntologyContext, RepairContext, SuggestionAnnotations, TypeAnnotation,
};
pub use explorer::{EntitySearchRequest, GraphExplorerService, MAX_SEARCH_FILTERS};
pub use graph_builder::{
    DEFAULT_AI_JOB_CONCURRENCY, GraphBuilderConfig, GraphBuilderResult, GraphBuilderService,
    MAX_AI_JOB_CONCURRENCY, MAX_REPAIR_ATTEMPTS, MAX_SUGGESTED_ENTITY_TYPES,
    MAX_SUGGESTED_PROPERTIES, MAX_SUGGESTED_RELATIONSHIP_TYPES, MAX_SUGGESTION_TEXT_CHARS,
    MAX_SUGGESTION_WARNINGS, SuggestionDataset, validate_suggestion,
};

pub use error::{ApplicationError, BoxError};
pub use graph_projects::{
    CreateGraphProject, DEFAULT_LIST_LIMIT, GraphProjectService, MAX_LIST_LIMIT,
};
pub use ingestion::{
    DEFAULT_BATCH_SIZE, DEFAULT_JOB_CONCURRENCY, DEFAULT_MAX_UPLOAD_BYTES, DEFAULT_QUEUE_CAPACITY,
    DataSourceView, IngestionConfig, IngestionPorts, IngestionService, MAX_BATCH_SIZE,
    MAX_IDEMPOTENCY_KEY_CHARS, PROCESS_RESTARTED, UploadSession,
};
pub use jobs::{
    IngestionJob, JobError, JobKind, JobResult, JobStatus, MAX_JOB_ERROR_CHARS,
    MAX_SUGGESTION_DATA_SOURCES,
};
pub use memory::MemoryMetadataStore;
pub use metadata::{DataSourceRecord, LOCAL_WORKSPACE_NAME, MetadataStore, MetadataStoreError};
pub use ontology::{
    EntityTypeInput, OntologyDefinition, OntologyService, PropertyInput, RelationshipTypeInput,
};
