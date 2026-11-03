//! Deterministic dataset ingestion for MindGraph Open (ADR-008).
//!
//! The engine underneath the (future, M5) AI Graph Builder. Everything here is
//! Rust-native and works without an LLM or the Python AI service:
//!
//! ```text
//! SourceStore ──► records (CSV / flat JSON, streamed) ──► DatasetProfile
//!                         │
//!                         ▼
//!         IngestionMapping ──validate against Ontology──► ValidatedMapping
//!                         │
//!                         ▼
//!     run_ingestion: preflight pass, then batched GraphStore writes
//! ```
//!
//! Orchestration (jobs, metadata, progress persistence) lives in
//! `mindgraph-core`; this crate has no dependency on it.

#![warn(missing_docs)]

mod engine;
mod error;
mod mapping;
mod profile;
mod record;
mod source;

pub use engine::{IngestionRun, IngestionSummary, ProgressSink, run_ingestion};
pub use error::IngestionError;
pub use mapping::{
    EntityMapping, EntityMode, FieldMapping, IngestionMapping, RelationshipMapping,
    ValidatedMapping, validate_mapping,
};
pub use profile::{
    DatasetProfile, FieldProfile, MAX_SAMPLE_ROWS, MAX_SAMPLE_VALUES, infer_type, profile_dataset,
    widen,
};
pub use record::{FieldValue, Limits};
pub use source::{LocalSourceStore, MemorySourceStore, SourceStore, SourceWriter, UploadDigest};
