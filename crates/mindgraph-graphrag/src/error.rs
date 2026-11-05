//! Errors of plan validation, evidence construction and retrieval.
//!
//! Messages name schema names, limits and MindGraph ids only: never
//! property values of stored entities.

use mindgraph_domain::RelationshipId;
use mindgraph_graph::GraphStoreError;

/// A retrieval intent that cannot become a plan.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// The intent is malformed or does not fit the ontology.
    #[error("invalid retrieval plan: {0}")]
    Invalid(String),
    /// The intent asks for more than the limits allow (for example a depth
    /// beyond the hard maximum).
    #[error("retrieval limit exceeded: {0}")]
    LimitExceeded(String),
}

/// An evidence graph invariant was broken. This indicates a bug or a
/// GraphStore that violates its contract, never a client mistake.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    /// An entity or relationship of another graph.
    #[error("{kind} {id} belongs to another graph")]
    ForeignGraph {
        /// `entity` or `relationship`.
        kind: &'static str,
        /// Its MindGraph id.
        id: String,
    },
    /// An entity or relationship listed twice.
    #[error("{kind} {id} is listed twice")]
    Duplicate {
        /// `entity` or `relationship`.
        kind: &'static str,
        /// Its MindGraph id.
        id: String,
    },
    /// A relationship whose endpoint is not in the evidence graph.
    #[error("relationship {0} has an endpoint outside the evidence graph")]
    MissingEndpoint(RelationshipId),
    /// A path names an entity or relationship that is not in the evidence
    /// graph.
    #[error("a path refers to {0}, which is not in the evidence graph")]
    UnknownPathItem(String),
    /// A path whose relationships do not connect its consecutive entities.
    #[error("an evidence path is not connected")]
    BrokenPath,
}

/// A failed retrieval.
#[derive(Debug, thiserror::Error)]
pub enum RetrievalError {
    /// The GraphStore failed.
    #[error(transparent)]
    Graph(#[from] GraphStoreError),
    /// The retrieved data broke an evidence invariant.
    #[error(transparent)]
    Evidence(#[from] EvidenceError),
}
