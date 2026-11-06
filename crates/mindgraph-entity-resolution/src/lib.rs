//! Basic entity resolution for MindGraph Open (ADR-009, M9).
//!
//! Deterministic, explainable algorithms for finding *potential* duplicate
//! entities: two records of the same entity type that may represent the same
//! real-world object. Everything here is pure Rust with no I/O; the
//! application layer (`mindgraph-core`) fetches entities and persists
//! results.
//!
//! ```text
//! entities of one type ─► normalize ─► block ─► compare ─► score ─► candidates
//!                         (rules.rs)   (blocking.rs)  (matching.rs)  (scoring.rs)
//! ```
//!
//! - [`RuleSet::validate`] checks the configured rules against the active
//!   ontology: `exact_identifier`, `normalized_email`, `normalized_phone`,
//!   `normalized_string` and `fuzzy_name`, each on one explicit property.
//! - [`find_candidates`] runs a bounded scan ([`ScanLimits`]) and returns
//!   [`CandidateMatch`]es with per-property evidence ([`MatchSignal`]) and a
//!   deterministic ranking score. The score orders review work; it is not a
//!   probability.
//! - [`plan_merge`] plans how an approved pair consolidates flat
//!   [`IdentityGroup`]s under the canonical entity a reviewer chose.
//!
//! Missing values never match, uncertain fuzzy matches are never approved
//! automatically, and no algorithm here changes an entity: identity is
//! recorded as metadata beside the graph, never as a graph rewrite.

#![warn(missing_docs)]

mod blocking;
mod candidate;
mod error;
mod identity;
mod matching;
mod normalize;
mod rules;
mod scoring;
mod similarity;

pub use blocking::NAME_BLOCK_PREFIX;
pub use candidate::{CandidateMatch, MatchSignal, SignalNote, SignalOutcome, ordered_pair};
pub use error::{MergeError, RuleError};
pub use identity::{DEFAULT_MAX_GROUP_MEMBERS, IdentityGroup, MergePlan, plan_merge};
pub use matching::{
    ScanLimits, ScanReport, ScanWarning, WarningCode, compare_entities, find_candidates,
};
pub use normalize::{
    Country, EmailKey, MAX_TEXT_CHARS, NameKey, Normalized, name_key, normalize_email,
    normalize_phone, normalize_text,
};
pub use rules::{
    DEFAULT_FUZZY_THRESHOLD, MAX_FUZZY_THRESHOLD, MAX_RULES, MIN_FUZZY_THRESHOLD, MatchRule,
    RuleKind, RuleSet, RuleSpec,
};
pub use scoring::{
    EMAIL_CASE_INSENSITIVE_WEIGHT, EMAIL_WEIGHT, EXACT_IDENTIFIER_WEIGHT, NAME_WEIGHT,
    PHONE_WEIGHT, STRING_WEIGHT, combine_weights,
};
pub use similarity::{INITIAL_SIMILARITY, NameSimilarity, name_similarity};
