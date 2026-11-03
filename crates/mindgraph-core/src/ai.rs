//! The AI Graph Builder port (ADR-007, M5).
//!
//! The application asks an AI service for a *suggestion* over one or more
//! analysed datasets: one ontology draft plus one M4 [`IngestionMapping`] per
//! dataset. AI suggests, Rust validates, the user reviews, and the
//! deterministic M4 engine executes, one dataset at a time. Nothing behind
//! this port can write graph data, ontologies or mappings.
//!
//! The port carries no transport types: the HTTP adapter lives in the
//! `mindgraph-ai-client` crate.

use async_trait::async_trait;
use mindgraph_domain::DataSourceId;
use mindgraph_ingestion::{DatasetProfile, IngestionMapping};
use serde::{Deserialize, Serialize};

use crate::OntologyDefinition;

/// Whether the AI Graph Builder can be used right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiStatus {
    /// A provider is configured and the AI service answers.
    Ready {
        /// The provider kind, for example `openai_compatible`.
        provider: String,
        /// The configured model name.
        model: String,
    },
    /// No provider is configured (AI is disabled).
    NotConfigured,
    /// The AI service cannot be reached.
    Unavailable,
}

/// A failure of the AI Graph Builder. Messages never contain dataset
/// values, prompts, provider responses or credentials.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    /// AI is disabled.
    #[error("the AI Graph Builder is not configured")]
    NotConfigured,
    /// The AI service or the provider cannot be reached.
    #[error("the AI service is unavailable: {0}")]
    Unavailable(String),
    /// The AI service or the provider did not answer in time.
    #[error("the AI provider did not answer in time")]
    Timeout,
    /// The provider refused the request (authentication, bad request).
    #[error("the AI provider rejected the request: {0}")]
    ProviderRejected(String),
    /// The provider is rate limiting.
    #[error("the AI provider is rate limiting requests")]
    RateLimited,
    /// The AI service or the model returned something that is not a valid
    /// structured suggestion.
    #[error("the AI service returned an invalid response: {0}")]
    InvalidResponse(String),
    /// The suggestion was well-formed but broke a MindGraph rule, even after
    /// the bounded repair attempts.
    #[error("the AI suggestion is invalid: {0}")]
    ValidationFailed(String),
    /// An unexpected integration failure.
    #[error("AI integration error: {0}")]
    Internal(String),
}

impl AiError {
    /// A stable, machine-readable code, for example `AI_TIMEOUT`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotConfigured => "AI_NOT_CONFIGURED",
            Self::Unavailable(_) => "AI_UNAVAILABLE",
            Self::Timeout => "AI_TIMEOUT",
            Self::ProviderRejected(_) => "AI_PROVIDER_REJECTED",
            Self::RateLimited => "AI_RATE_LIMITED",
            Self::InvalidResponse(_) => "AI_INVALID_RESPONSE",
            Self::ValidationFailed(_) => "AI_VALIDATION_FAILED",
            Self::Internal(_) => "AI_INTERNAL_ERROR",
        }
    }
}

/// The active ontology given to the AI as context: its version and types
/// (without ids, which are irrelevant to inference).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyContext {
    /// The pinned ontology version the suggestion builds on.
    pub version: u32,
    /// Its types.
    #[serde(flatten)]
    pub definition: OntologyDefinition,
}

/// A previous suggestion and why Rust rejected it, for one repair attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepairContext {
    /// The rejected suggestion.
    pub previous: GraphBuilderSuggestion,
    /// Concise validation errors (no internal details).
    pub errors: Vec<String>,
}

/// One analysed dataset as the AI sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetContext {
    /// A label unique within the request, derived from the uploaded
    /// filename (for example `customers.csv`). Suggestions refer to datasets
    /// by this label.
    pub name: String,
    /// The M4 profile: field names, inferred types, nullability and bounded
    /// samples.
    pub profile: DatasetProfile,
}

/// Everything the AI receives: bounded, structured context only. Never the
/// source files, filesystem paths, graph contents or credentials.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphBuilderRequest {
    /// Propagated as the request id (the job id).
    pub correlation_id: String,
    /// The datasets to model together, in the order the client gave them.
    pub datasets: Vec<DatasetContext>,
    /// The pinned active ontology.
    pub active_ontology: OntologyContext,
    /// Set for a repair attempt.
    pub repair: Option<RepairContext>,
}

/// Confidence and a short reason for one suggested type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeAnnotation {
    /// The entity or relationship type name.
    pub name: String,
    /// Model confidence, `0.0..=1.0`. Not a statistical guarantee.
    pub confidence: f64,
    /// A short human-readable reason.
    #[serde(default)]
    pub reason: String,
    /// Whether the type already exists in the base ontology. Always set by
    /// Rust validation; whatever the AI claims is ignored.
    #[serde(default)]
    pub existing: bool,
}

/// Per-type annotations of a suggestion.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestionAnnotations {
    /// Annotations of entity types.
    #[serde(default)]
    pub entities: Vec<TypeAnnotation>,
    /// Annotations of relationship types.
    #[serde(default)]
    pub relationships: Vec<TypeAnnotation>,
}

/// The M4 mapping of one dataset of a suggestion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetMapping {
    /// The dataset label from the request.
    pub dataset: String,
    /// The data source the label stands for. Set by Rust validation, never
    /// by the AI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_source_id: Option<DataSourceId>,
    /// How each record of the dataset maps onto the proposed ontology.
    pub mapping: IngestionMapping,
}

/// An AI Graph Builder suggestion. `ontology` is the complete proposed next
/// version (the same shape as `POST /graphs/{id}/ontology`); `mappings` holds
/// one unchanged M4 ingestion mapping per dataset, in a safe ingestion order
/// once validated (datasets that create a type before datasets that only
/// match it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SuggestionWire")]
pub struct GraphBuilderSuggestion {
    /// The proposed ontology: existing types kept, new types added.
    pub ontology: OntologyDefinition,
    /// One mapping per dataset.
    pub mappings: Vec<DatasetMapping>,
    /// Overall model confidence, `0.0..=1.0`.
    pub confidence: f64,
    /// Per-type confidence and reasons.
    #[serde(default)]
    pub annotations: SuggestionAnnotations,
    /// Short, bounded warnings for the reviewer.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// The accepted wire shapes of a suggestion: `mappings`, or the single
/// `mapping` of suggestions stored before multi-dataset support (its data
/// source is the job's first one; the store fills it in).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SuggestionWire {
    ontology: OntologyDefinition,
    #[serde(default)]
    mappings: Option<Vec<DatasetMapping>>,
    #[serde(default)]
    mapping: Option<IngestionMapping>,
    confidence: f64,
    #[serde(default)]
    annotations: SuggestionAnnotations,
    #[serde(default)]
    warnings: Vec<String>,
}

impl TryFrom<SuggestionWire> for GraphBuilderSuggestion {
    type Error = &'static str;

    fn try_from(wire: SuggestionWire) -> Result<Self, Self::Error> {
        let mappings = match (wire.mappings, wire.mapping) {
            (Some(mappings), None) => mappings,
            (None, Some(mapping)) => vec![DatasetMapping {
                dataset: String::new(),
                data_source_id: None,
                mapping,
            }],
            (Some(_), Some(_)) => return Err("a suggestion has either `mappings` or `mapping`"),
            (None, None) => return Err("a suggestion needs `mappings`"),
        };
        Ok(Self {
            ontology: wire.ontology,
            mappings,
            confidence: wire.confidence,
            annotations: wire.annotations,
            warnings: wire.warnings,
        })
    }
}

/// Where a suggestion came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiMetadata {
    /// The prompt version, for example `graph_builder_v1`.
    pub prompt_version: String,
    /// The provider kind.
    pub provider: String,
    /// The model name.
    pub model: String,
}

/// An unvalidated suggestion as returned by the AI service.
#[derive(Debug, Clone, PartialEq)]
pub struct AiSuggestion {
    /// The suggestion, still to be validated by Rust.
    pub suggestion: GraphBuilderSuggestion,
    /// Provenance.
    pub metadata: AiMetadata,
}

/// The AI Graph Builder port. Object safe: use `Arc<dyn GraphBuilderAi>`.
#[async_trait]
pub trait GraphBuilderAi: Send + Sync {
    /// Whether a suggestion can be requested now. Cheap; called before a job
    /// is scheduled.
    async fn status(&self) -> AiStatus;

    /// Requests one suggestion. Callers validate the result.
    ///
    /// # Errors
    ///
    /// An [`AiError`] category.
    async fn suggest(&self, request: &GraphBuilderRequest) -> Result<AiSuggestion, AiError>;
}

/// The port used when AI is disabled (no `MINDGRAPH_AI_URL`).
#[derive(Debug, Clone, Copy, Default)]
pub struct DisabledGraphBuilderAi;

#[async_trait]
impl GraphBuilderAi for DisabledGraphBuilderAi {
    async fn status(&self) -> AiStatus {
        AiStatus::NotConfigured
    }

    async fn suggest(&self, _: &GraphBuilderRequest) -> Result<AiSuggestion, AiError> {
        Err(AiError::NotConfigured)
    }
}
