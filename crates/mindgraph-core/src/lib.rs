//! Application layer of MindGraph Open (ADR-002).
//!
//! `mindgraph-core` runs use cases on top of `mindgraph-domain`. It reaches
//! infrastructure only through ports, so every entry point shares one
//! implementation of every use case (entity resolution matching lives in the
//! pure `mindgraph-entity-resolution` crate). The MCP server (`mindgraph-mcp`) uses
//! them through the HTTP API, not as a crate dependency (ADR-011, M8):
//!
//! ```text
//! mindgraph-api / mindgraph-cli
//!                  │
//!                  ▼
//!            mindgraph-core ── MetadataStore (port) ◄── mindgraph-metadata (PostgreSQL)
//!                  │        ── GraphStore (port)    ◄── HugeGraphStore (mindgraph-graph)
//!                  │        ── SourceStore (port)   ◄── LocalSourceStore (mindgraph-ingestion)
//!                  │        ── GraphBuilderAi (port) ◄── HttpGraphBuilderAi (mindgraph-ai-client)
//!                  │        ── GraphRagAi (port)     ◄── HttpGraphRagAi (mindgraph-ai-client)
//!                  │        ── EntityResolutionStore (port) ◄── PostgresEntityResolutionStore (mindgraph-metadata)
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
mod graphrag;
mod ingestion;
mod jobs;
mod memory;
mod metadata;
mod ontology;
mod resolution;
mod resolution_memory;
mod resolution_store;

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
pub use graphrag::{
    AnswerMetadata, AnswerSource, AnswerStatus, AskQuestion, AskScope, ConversationTurn,
    ConversationTurnContext, DEFAULT_GRAPHRAG_CONCURRENCY, DEFAULT_GRAPHRAG_TIMEOUT,
    DisabledGraphRagAi, GraphRagAi, GraphRagAnswer, GraphRagConfig, GraphRagError, GraphRagService,
    Interpretation, InterpretationLimits, InterpretationRequest, KnownEntityContext,
    MAX_GRAPHRAG_CONCURRENCY, MAX_GRAPHRAG_TIMEOUT, RetrievalCompleteness, RetrievalReport,
    Synthesis, SynthesisRequest, SynthesizedAnswer,
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
/// The pure matching vocabulary (rules, evidence, identity groups) used by
/// the resolution use cases, re-exported for adapters and transports.
pub use mindgraph_entity_resolution as matching;
pub use ontology::{
    EntityTypeInput, OntologyDefinition, OntologyService, PropertyInput, RelationshipTypeInput,
};
pub use resolution::{
    CandidateDetail, DEFAULT_CANDIDATE_PAGE, DEFAULT_SCAN_CONCURRENCY, DEFAULT_SCAN_ENTITIES,
    DEFAULT_SCAN_TIMEOUT, DecisionRequest, DecisionResult, EntityResolutionConfig,
    EntityResolutionService, IdentityView, MAX_CANDIDATE_PAGE, MAX_SCAN_ENTITIES, MAX_SCAN_HISTORY,
    MIN_SCAN_ENTITIES, ResolutionError, ScanRequest,
};
pub use resolution_memory::MemoryEntityResolutionStore;
pub use resolution_store::{
    CandidateCounts, CandidatePage, CandidateQuery, CandidateStatus, DecisionCommand, DecisionKind,
    DecisionOutcome, EntityResolutionStore, IdentityMembership, LOCAL_ACTOR, NewCandidate, Repeat,
    ResolutionCandidate, ResolutionDecision, ResolutionScan, ResolutionStoreError, SkippedPairs,
    StoredIdentityGroup, check_new_candidates, check_repeat, decision_record, linked_pair_conflict,
    linked_sides, plan_same_entity, rejected_pair_conflict,
};
