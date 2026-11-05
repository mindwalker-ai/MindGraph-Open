//! Graph-first GraphRAG retrieval for MindGraph Open (ADR-010, M7).
//!
//! The Rust side of the GraphRAG pipeline. Natural-language interpretation
//! and answer synthesis happen in the Python AI service; everything that
//! touches graph data happens here, through the `GraphStore` port:
//!
//! ```text
//! RetrievalIntent (untrusted model output)
//!        │ validate_intent: ontology names, typed values, ids, depths
//!        ▼
//! RetrievalPlan ──► retrieve: bounded GraphStore calls (search, neighbours, path)
//!        │                 │ operation budget, evidence budget, id order
//!        ▼                 ▼
//!               EvidenceGraph (real entities and relationships, E1/R1/P1)
//!                          │ build_context: compact JSON within 48 KiB
//!                          ▼
//!                 model context ──► synthesis ──► resolve_citations
//! ```
//!
//! Four bounded operations are supported: `entity_lookup`, `neighbors`,
//! `find_path` and `shared_neighbor` (plus `unsupported`). There is no query
//! language: the model never writes Gremlin, Cypher or SQL, never reaches
//! the GraphStore, and nothing here writes graph data. An answer's evidence
//! only ever contains what was read, and a citation is only valid if it
//! names a retrieved object.

#![warn(missing_docs)]

mod context;
mod error;
mod evidence;
mod limits;
mod plan;
mod retrieval;
mod validation;

pub use context::{ContextMeta, SerializedContext, build_context, model_properties};
pub use error::{EvidenceError, PlanError, RetrievalError};
pub use evidence::{
    Citation, CitationCheck, CitedObject, EvidenceBuilder, EvidenceGraph, EvidencePath,
    EvidenceRef, RefKind, SharedGroup,
};
pub use limits::{
    GraphRagLimits, MAX_AMBIGUOUS_CANDIDATES, MAX_CONVERSATION_TURNS, MAX_EARLIER_ENTITIES,
    MAX_MATCH_PROPERTIES, MAX_QUESTION_CHARS, MAX_SEED_ENTITIES, MAX_TURN_ENTITIES,
};
pub use plan::{
    Anchor, AnchorIntent, KnownRef, Operation, PlanSummary, RetrievalIntent, RetrievalPlan,
    ValidatedPlan,
};
pub use retrieval::{Resolution, RetrievalOutcome, RetrievalStats, retrieve};
pub use validation::{PlanContext, validate_intent};
