//! GraphRAG (ADR-010, M7): graph-first questions with verifiable evidence.
//!
//! ```text
//! mindgraph-api ─► GraphRagService
//!                    1. graph exists (workspace) · question, scope and history valid
//!                    2. active ontology · selected entities and the entities of
//!                       earlier questions (follow-ups) exist in this graph
//!                    3. GraphRagAi::interpret ──► RetrievalIntent (untrusted)
//!                    4. validate_intent ──► RetrievalPlan (mindgraph-graphrag)
//!                    5. retrieve ──► GraphStore (bounded) ──► EvidenceGraph
//!                    6. build_context (≤ 48 KiB, references only)
//!                    7. GraphRagAi::synthesize ──► answer + cited references
//!                    8. citations checked against the evidence · status
//! ```
//!
//! Python interprets and phrases; Rust validates every name, id and depth,
//! owns every graph read and decides which references are real. The model
//! never reaches the GraphStore and nothing here writes graph data.
//!
//! Follow-up questions carry up to three earlier questions of the same
//! conversation, held by the client: their text and the entity ids they
//! cited. Nothing is stored on the server, the ids are checked against this
//! graph again, and evidence is always retrieved afresh, so an earlier
//! answer is never reused as evidence. Each
//! question runs under a deadline and a concurrency limit and holds no
//! database transaction while it waits for the model. Logs carry ids,
//! counts, sizes and durations: never the question, property values or
//! model output.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mindgraph_domain::{Entity, EntityId, GraphId, Ontology};
use mindgraph_graph::GraphStoreError;
use mindgraph_graphrag::{
    Citation, ContextMeta, EvidenceGraph, EvidenceRef, GraphRagLimits, MAX_CONVERSATION_TURNS,
    MAX_EARLIER_ENTITIES, MAX_QUESTION_CHARS, MAX_SEED_ENTITIES, MAX_TURN_ENTITIES, PlanContext,
    PlanError, PlanSummary, Resolution, RetrievalError, RetrievalIntent, RetrievalPlan,
    build_context, model_properties, retrieve, validate_intent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;
use tracing::{info, warn};

use crate::explorer::graph_failure;
use crate::{
    AiError, AiMetadata, AiStatus, ApplicationError, GraphExplorerService, OntologyContext,
    OntologyDefinition,
};

/// Default deadline of one question: two model calls (each bounded by the
/// AI service's provider timeout, 60 s by default) plus bounded retrieval.
pub const DEFAULT_GRAPHRAG_TIMEOUT: Duration = Duration::from_secs(120);
/// Longest accepted deadline.
pub const MAX_GRAPHRAG_TIMEOUT: Duration = Duration::from_secs(600);
/// Default number of questions answered at the same time.
pub const DEFAULT_GRAPHRAG_CONCURRENCY: usize = 2;
/// Largest accepted concurrency.
pub const MAX_GRAPHRAG_CONCURRENCY: usize = 16;

/// Longest answer kept, in characters.
const MAX_ANSWER_CHARS: usize = 4_000;
/// Most model warnings kept.
const MAX_WARNINGS: usize = 10;
/// Longest model warning kept, in characters.
const MAX_WARNING_CHARS: usize = 300;

// --- errors ------------------------------------------------------------------------------

/// Why a GraphRAG question failed. A question without evidence is not a
/// failure: it is answered with `insufficient_evidence`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GraphRagError {
    /// The question is empty or too long.
    #[error("invalid question: {0}")]
    InvalidQuestion(String),
    /// The scope is malformed (for example too many selected entities).
    #[error("invalid scope: {0}")]
    InvalidScope(String),
    /// The conversation history of a follow-up is malformed.
    #[error("invalid history: {0}")]
    InvalidHistory(String),
    /// The model's retrieval plan is invalid for this graph.
    #[error("invalid retrieval plan: {0}")]
    InvalidRetrievalPlan(String),
    /// The model's retrieval plan exceeds the limits.
    #[error("retrieval limit exceeded: {0}")]
    RetrievalLimitExceeded(String),
    /// No model is configured.
    #[error("AI is not configured")]
    AiNotConfigured,
    /// The AI service or the provider is unavailable (or rate limiting).
    #[error("the AI service is unavailable")]
    AiUnavailable,
    /// The question did not finish within the deadline.
    #[error("the question was not answered within {0} seconds")]
    Timeout(u64),
    /// The AI provider did not answer in time.
    #[error("the AI provider did not answer in time")]
    AiTimeout,
    /// The model's answer was not a valid structured answer.
    #[error("the model returned an invalid answer")]
    AiInvalidResponse,
}

impl From<GraphRagError> for ApplicationError {
    fn from(error: GraphRagError) -> Self {
        Self::GraphRag(error)
    }
}

// --- the AI port ---------------------------------------------------------------------------

/// A known entity as the model sees it: a reference (`S1` for a selected
/// entity, `H1` for an entity of an earlier question), its type and bounded
/// properties. Never its MindGraph id.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnownEntityContext {
    /// `S1`, `S2`, … or `H1`, `H2`, …
    #[serde(rename = "ref")]
    pub reference: String,
    /// The entity type.
    pub entity_type: String,
    /// Bounded properties.
    pub properties: Value,
}

/// The bounds the interpretation must respect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InterpretationLimits {
    /// Deepest neighbourhood traversal.
    pub max_traversal_depth: usize,
    /// Deepest path search.
    pub max_path_depth: usize,
}

/// An earlier question of the conversation, as the model sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversationTurnContext {
    /// The earlier question (untrusted).
    pub question: String,
    /// References (`S1`, `H1`, …) of the entities it cited that still exist.
    pub entities: Vec<String>,
}

/// Everything the interpretation receives: the question, the ontology, the
/// selected entities and, for a follow-up, the earlier questions and their
/// entities. Never other graph data or credentials.
#[derive(Debug, Clone, PartialEq)]
pub struct InterpretationRequest {
    /// Propagated as the request id.
    pub correlation_id: String,
    /// The user's question (untrusted).
    pub question: String,
    /// The active ontology.
    pub ontology: OntologyContext,
    /// The selected entities, `S1` first.
    pub selected_entities: Vec<KnownEntityContext>,
    /// Earlier questions of the conversation, oldest first (follow-ups).
    pub conversation: Vec<ConversationTurnContext>,
    /// Entities cited by earlier questions that are not selected, `H1` first.
    pub earlier_entities: Vec<KnownEntityContext>,
    /// The bounds.
    pub limits: InterpretationLimits,
}

/// An interpretation, still to be validated.
#[derive(Debug, Clone, PartialEq)]
pub struct Interpretation {
    /// The structured intent (untrusted).
    pub intent: RetrievalIntent,
    /// Provenance.
    pub metadata: AiMetadata,
}

/// What the retrieval covered, for the synthesis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetrievalCompleteness {
    /// Whether the requested scope was read completely.
    pub complete: bool,
    /// Whether a limit stopped the retrieval or reduced the context.
    pub truncated: bool,
    /// User-visible limitations.
    pub limitations: Vec<String>,
}

/// Everything the synthesis receives: the question and the bounded evidence
/// context (references, types and properties; no MindGraph ids).
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesisRequest {
    /// Propagated as the request id.
    pub correlation_id: String,
    /// The user's question (untrusted).
    pub question: String,
    /// Earlier questions of the conversation, oldest first: only to
    /// understand what the question refers to, never evidence.
    pub earlier_questions: Vec<String>,
    /// The serialised evidence context.
    pub context: Value,
    /// What the retrieval covered.
    pub retrieval: RetrievalCompleteness,
}

/// A structured answer, still to be checked against the evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesizedAnswer {
    /// The answer text; may cite references such as `[E1]`.
    pub answer: String,
    /// The references the answer relies on.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// The model judged the evidence insufficient.
    #[serde(default)]
    pub insufficient_evidence: bool,
    /// Short caveats.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// A synthesis and its provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct Synthesis {
    /// The answer.
    pub answer: SynthesizedAnswer,
    /// Provenance.
    pub metadata: AiMetadata,
}

/// The GraphRAG AI port (ADR-007): natural-language interpretation and
/// synthesis only. Nothing behind it can read or write graph data. Object
/// safe: use `Arc<dyn GraphRagAi>`.
#[async_trait]
pub trait GraphRagAi: Send + Sync {
    /// Whether questions can be answered now. Cheap.
    async fn status(&self) -> AiStatus;

    /// Interprets a question into a retrieval intent.
    ///
    /// # Errors
    ///
    /// An [`AiError`] category.
    async fn interpret(&self, request: &InterpretationRequest) -> Result<Interpretation, AiError>;

    /// Phrases an answer from the evidence context.
    ///
    /// # Errors
    ///
    /// An [`AiError`] category.
    async fn synthesize(&self, request: &SynthesisRequest) -> Result<Synthesis, AiError>;
}

/// The port used when AI is disabled: every question is
/// `AI_NOT_CONFIGURED`; exploration keeps working.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisabledGraphRagAi;

#[async_trait]
impl GraphRagAi for DisabledGraphRagAi {
    async fn status(&self) -> AiStatus {
        AiStatus::NotConfigured
    }

    async fn interpret(&self, _: &InterpretationRequest) -> Result<Interpretation, AiError> {
        Err(AiError::NotConfigured)
    }

    async fn synthesize(&self, _: &SynthesisRequest) -> Result<Synthesis, AiError> {
        Err(AiError::NotConfigured)
    }
}

// --- requests and answers --------------------------------------------------------------------

/// Settings of the GraphRAG service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphRagConfig {
    /// Retrieval and context limits.
    pub limits: GraphRagLimits,
    /// Deadline of one question, including waiting for a slot.
    pub timeout: Duration,
    /// Questions answered at the same time.
    pub concurrency: usize,
}

impl Default for GraphRagConfig {
    fn default() -> Self {
        Self {
            limits: GraphRagLimits::default(),
            timeout: DEFAULT_GRAPHRAG_TIMEOUT,
            concurrency: DEFAULT_GRAPHRAG_CONCURRENCY,
        }
    }
}

/// What a question is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskScope {
    /// The selected entities (`1..=5`), the most reliable mode.
    SelectedEntities(Vec<EntityId>),
    /// Bounded discovery over the graph: never an unlimited scan.
    BoundedGraph,
}

impl AskScope {
    /// `selected_entities` or `bounded_graph`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::SelectedEntities(_) => "selected_entities",
            Self::BoundedGraph => "bounded_graph",
        }
    }
}

/// An earlier question of the conversation, sent by the client with a
/// follow-up. Untrusted: the ids are checked against the graph again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurn {
    /// The earlier question.
    pub question: String,
    /// Entities that question was about or cited, most relevant first.
    pub entity_ids: Vec<EntityId>,
}

/// One question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskQuestion {
    /// The question (untrusted text).
    pub question: String,
    /// Its scope.
    pub scope: AskScope,
    /// Earlier questions of the conversation, oldest first (at most
    /// [`MAX_CONVERSATION_TURNS`]); empty for a first question.
    pub history: Vec<ConversationTurn>,
    /// The request id, propagated to the AI service.
    pub correlation_id: String,
}

/// How well the evidence supports the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerStatus {
    /// Answered from evidence that covers the question's scope.
    Answered,
    /// Answered from evidence, but a limit stopped retrieval: the answer
    /// may miss results.
    Partial,
    /// The retrieved graph context does not support an answer.
    InsufficientEvidence,
}

impl AnswerStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Partial => "partial",
            Self::InsufficientEvidence => "insufficient_evidence",
        }
    }
}

/// Who wrote the answer text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerSource {
    /// The configured model, from the evidence context.
    Model,
    /// MindGraph: retrieval found no evidence, so the model was not asked.
    NoEvidence,
    /// MindGraph: the question does not map to a supported operation.
    Unsupported,
}

/// What retrieval did, for the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetrievalReport {
    /// `selected_entities` or `bounded_graph`.
    pub scope: &'static str,
    /// The validated plan.
    pub plan: PlanSummary,
    /// GraphStore calls.
    pub operations: usize,
    /// Candidates inspected.
    pub candidates_inspected: usize,
    /// Whether a limit stopped retrieval or reduced the context.
    pub truncated: bool,
    /// Size of the model context, in bytes (0 when the model was not asked).
    pub context_bytes: usize,
    /// Limitations of this result.
    pub limitations: Vec<String>,
}

/// Provenance of an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnswerMetadata {
    /// The interpretation prompt version.
    pub interpret_prompt_version: String,
    /// The synthesis prompt version, when the model phrased the answer.
    pub synthesize_prompt_version: Option<String>,
    /// The provider kind.
    pub provider: String,
    /// The model name.
    pub model: String,
    /// Earlier questions used as conversation context.
    pub conversation_turns: usize,
    /// Total time, in milliseconds.
    pub duration_ms: u64,
}

/// A GraphRAG answer: text, the evidence it rests on, and checked citations.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphRagAnswer {
    /// How well the evidence supports the answer.
    pub status: AnswerStatus,
    /// The answer text, with unknown citations removed.
    pub answer: String,
    /// Who wrote it.
    pub source: AnswerSource,
    /// The evidence (exactly what the model saw).
    pub evidence: EvidenceGraph,
    /// Citations that refer to the evidence.
    pub citations: Vec<Citation>,
    /// Caveats (the model's, bounded, and MindGraph's).
    pub warnings: Vec<String>,
    /// What retrieval did.
    pub retrieval: RetrievalReport,
    /// Provenance.
    pub metadata: AnswerMetadata,
}

// --- the service ---------------------------------------------------------------------------------

/// Answers questions about one graph from bounded, verifiable evidence.
#[derive(Clone)]
pub struct GraphRagService {
    explorer: GraphExplorerService,
    ai: Arc<dyn GraphRagAi>,
    config: GraphRagConfig,
    permits: Arc<Semaphore>,
}

impl std::fmt::Debug for GraphRagService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphRagService")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn bounded(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(max_chars).collect();
    kept.push('…');
    kept
}

/// The question, trimmed.
fn check_question(raw: &str) -> Result<String, GraphRagError> {
    let question = raw.trim();
    if question.is_empty() {
        return Err(GraphRagError::InvalidQuestion(
            "the question must not be empty".to_owned(),
        ));
    }
    if question.chars().count() > MAX_QUESTION_CHARS {
        return Err(GraphRagError::InvalidQuestion(format!(
            "the question may have at most {MAX_QUESTION_CHARS} characters"
        )));
    }
    Ok(question.to_owned())
}

/// The selected entity ids, without repeats, in the given order.
fn check_scope(scope: &AskScope) -> Result<Vec<EntityId>, GraphRagError> {
    let AskScope::SelectedEntities(ids) = scope else {
        return Ok(Vec::new());
    };
    let mut unique = Vec::new();
    for id in ids {
        if !unique.contains(id) {
            unique.push(*id);
        }
    }
    if unique.is_empty() {
        return Err(GraphRagError::InvalidScope(
            "selected_entities needs at least one entity id".to_owned(),
        ));
    }
    if unique.len() > MAX_SEED_ENTITIES {
        return Err(GraphRagError::InvalidScope(format!(
            "a question may be scoped to at most {MAX_SEED_ENTITIES} entities"
        )));
    }
    Ok(unique)
}

/// The earlier questions, trimmed and bounded.
fn check_history(history: &[ConversationTurn]) -> Result<Vec<ConversationTurn>, GraphRagError> {
    if history.len() > MAX_CONVERSATION_TURNS {
        return Err(GraphRagError::InvalidHistory(format!(
            "a follow-up may carry at most {MAX_CONVERSATION_TURNS} earlier questions"
        )));
    }
    history
        .iter()
        .map(|turn| {
            let question = check_question(&turn.question).map_err(|_| {
                GraphRagError::InvalidHistory(format!(
                    "every earlier question needs 1 to {MAX_QUESTION_CHARS} characters"
                ))
            })?;
            if turn.entity_ids.len() > MAX_TURN_ENTITIES {
                return Err(GraphRagError::InvalidHistory(format!(
                    "an earlier question may carry at most {MAX_TURN_ENTITIES} entity ids"
                )));
            }
            let mut entity_ids = Vec::new();
            for id in &turn.entity_ids {
                if !entity_ids.contains(id) {
                    entity_ids.push(*id);
                }
            }
            Ok(ConversationTurn {
                question,
                entity_ids,
            })
        })
        .collect()
}

/// The entities of earlier questions a follow-up may refer to: newest
/// question first, without the selected ones, at most
/// [`MAX_EARLIER_ENTITIES`]. Returned in conversation order (oldest first),
/// which numbers them `H1`, `H2`, …
fn earlier_candidates(history: &[ConversationTurn], selected: &[EntityId]) -> Vec<EntityId> {
    let mut kept: Vec<EntityId> = Vec::new();
    'turns: for turn in history.iter().rev() {
        for id in &turn.entity_ids {
            if kept.len() == MAX_EARLIER_ENTITIES {
                break 'turns;
            }
            if !selected.contains(id) && !kept.contains(id) {
                kept.push(*id);
            }
        }
    }
    let mut ordered = Vec::with_capacity(kept.len());
    for turn in history {
        for id in &turn.entity_ids {
            if kept.contains(id) && !ordered.contains(id) {
                ordered.push(*id);
            }
        }
    }
    ordered
}

fn ai_failure(error: &AiError, stage: &'static str) -> ApplicationError {
    // Codes only: AI errors never carry data, but stay terse in logs anyway.
    warn!(stage, code = error.code(), "GraphRAG AI call failed");
    match error {
        AiError::NotConfigured => GraphRagError::AiNotConfigured.into(),
        AiError::Unavailable(_) | AiError::RateLimited | AiError::ProviderRejected(_) => {
            GraphRagError::AiUnavailable.into()
        }
        AiError::Timeout => GraphRagError::AiTimeout.into(),
        AiError::InvalidResponse(_) | AiError::ValidationFailed(_) if stage == "interpret" => {
            GraphRagError::InvalidRetrievalPlan(
                "the model did not return a valid retrieval plan; try rephrasing the question"
                    .to_owned(),
            )
            .into()
        }
        AiError::InvalidResponse(_) | AiError::ValidationFailed(_) => {
            GraphRagError::AiInvalidResponse.into()
        }
        AiError::Internal(message) => ApplicationError::internal(message.clone()),
    }
}

fn plan_failure(error: PlanError) -> ApplicationError {
    match error {
        PlanError::Invalid(message) => GraphRagError::InvalidRetrievalPlan(message).into(),
        PlanError::LimitExceeded(message) => GraphRagError::RetrievalLimitExceeded(message).into(),
    }
}

fn retrieval_failure(error: RetrievalError) -> ApplicationError {
    match error {
        RetrievalError::Graph(error @ GraphStoreError::BackendUnavailable { .. }) => {
            ApplicationError::unavailable(error)
        }
        other => ApplicationError::internal(other),
    }
}

/// Removes inline citations (`[E1]`, `[E1, R2]`) that do not refer to the
/// evidence and returns the cleaned text, the inline references and how
/// many were removed. Brackets that are not citation lists stay unchanged.
fn clean_inline_citations(text: &str, evidence: &EvidenceGraph) -> (String, Vec<String>, usize) {
    let mut out = String::with_capacity(text.len());
    let mut cited = Vec::new();
    let mut removed = 0;
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find(']').filter(|close| *close <= 64) else {
            out.push('[');
            rest = after;
            continue;
        };
        let inner = &after[..close];
        let tokens: Vec<&str> = inner
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|token| !token.is_empty())
            .collect();
        let looks_like_refs = !tokens.is_empty()
            && tokens.iter().all(|token| {
                let mut chars = token.chars();
                matches!(chars.next(), Some('E' | 'R' | 'P'))
                    && !chars.as_str().is_empty()
                    && chars.as_str().bytes().all(|b| b.is_ascii_digit())
            });
        if !looks_like_refs {
            out.push('[');
            rest = after;
            continue;
        }
        let valid: Vec<&str> = tokens
            .iter()
            .copied()
            .filter(|token| {
                token
                    .parse::<EvidenceRef>()
                    .ok()
                    .and_then(|reference| evidence.resolve(reference))
                    .is_some()
            })
            .collect();
        removed += tokens.len() - valid.len();
        cited.extend(valid.iter().map(|token| (*token).to_owned()));
        if valid.is_empty() {
            // Drop the marker and the space before it.
            while out.ends_with(' ') {
                out.pop();
            }
        } else {
            out.push('[');
            out.push_str(&valid.join(", "));
            out.push(']');
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    (out.trim().to_owned(), cited, removed)
}

/// A checked question, ready to be answered.
struct Prepared {
    /// The question, trimmed.
    text: String,
    ontology: Ontology,
    /// `S1` first.
    selected: Vec<Entity>,
    /// Earlier questions, oldest first.
    history: Vec<ConversationTurn>,
    /// Entities of earlier questions that still exist, `H1` first.
    earlier: Vec<Entity>,
    /// Entities of earlier questions that are not in this graph any more.
    missing: usize,
}

impl Prepared {
    fn known_ref(&self, id: EntityId) -> Option<String> {
        if let Some(index) = self.selected.iter().position(|e| e.id() == id) {
            return Some(format!("S{}", index + 1));
        }
        self.earlier
            .iter()
            .position(|e| e.id() == id)
            .map(|index| format!("H{}", index + 1))
    }
}

fn known_entity(reference: String, entity: &Entity, limits: &GraphRagLimits) -> KnownEntityContext {
    KnownEntityContext {
        reference,
        entity_type: entity.entity_type().to_string(),
        properties: model_properties(entity.properties(), limits),
    }
}

impl GraphRagService {
    /// A service reading graphs through `explorer` (which owns workspace and
    /// graph isolation) and asking `ai` to interpret and phrase.
    #[must_use]
    pub fn new(
        explorer: GraphExplorerService,
        ai: Arc<dyn GraphRagAi>,
        config: GraphRagConfig,
    ) -> Self {
        let config = GraphRagConfig {
            limits: config.limits.sanitized(),
            timeout: config
                .timeout
                .clamp(Duration::from_secs(1), MAX_GRAPHRAG_TIMEOUT),
            concurrency: config.concurrency.clamp(1, MAX_GRAPHRAG_CONCURRENCY),
        };
        Self {
            explorer,
            ai,
            permits: Arc::new(Semaphore::new(config.concurrency)),
            config,
        }
    }

    /// The effective settings.
    #[must_use]
    pub const fn config(&self) -> &GraphRagConfig {
        &self.config
    }

    /// Whether questions can be answered now.
    pub async fn status(&self) -> AiStatus {
        self.ai.status().await
    }

    /// Answers a question about one graph.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ApplicationError::EntityNotFound`]
    /// for a selected entity that is not in the graph, a
    /// [`GraphRagError`] (invalid question, scope or plan, AI failures, the
    /// deadline), or a backend failure. Missing evidence is not an error.
    pub async fn ask(
        &self,
        graph_id: GraphId,
        question: AskQuestion,
    ) -> Result<GraphRagAnswer, ApplicationError> {
        let started = Instant::now();
        let text = check_question(&question.question)?;
        let seeds = check_scope(&question.scope)?;
        let history = check_history(&question.history)?;
        self.explorer.ensure_graph(graph_id).await?;
        let ontology = self.explorer.active_ontology(graph_id).await?;
        let mut selected = Vec::with_capacity(seeds.len());
        for id in &seeds {
            selected.push(self.explorer.get_entity(graph_id, *id).await?);
        }
        // Entities of earlier questions are checked against this graph again;
        // one that is gone (or never was here) is ignored, not an error.
        let mut earlier = Vec::new();
        let mut missing = 0;
        for id in earlier_candidates(&history, &seeds) {
            match self
                .explorer
                .graph_store()
                .get_entity(graph_id, id)
                .await
                .map_err(graph_failure)?
            {
                Some(entity) => earlier.push(entity),
                None => missing += 1,
            }
        }
        let prepared = Prepared {
            text,
            ontology,
            selected,
            history,
            earlier,
            missing,
        };
        let deadline = self.config.timeout;
        let answer = tokio::time::timeout(
            deadline,
            self.answer(graph_id, &question, &prepared, started),
        )
        .await;
        match answer {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    graph_id = %graph_id,
                    request_id = %question.correlation_id,
                    timeout_s = deadline.as_secs(),
                    "GraphRAG question timed out"
                );
                Err(GraphRagError::Timeout(deadline.as_secs()).into())
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn answer(
        &self,
        graph_id: GraphId,
        question: &AskQuestion,
        prepared: &Prepared,
        started: Instant,
    ) -> Result<GraphRagAnswer, ApplicationError> {
        let Prepared {
            text,
            ontology,
            selected,
            history,
            earlier,
            missing,
        } = prepared;
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(ApplicationError::internal)?;
        let limits = self.config.limits;
        let scope = question.scope.as_str();

        let interpretation = self
            .ai
            .interpret(&InterpretationRequest {
                correlation_id: question.correlation_id.clone(),
                question: text.clone(),
                ontology: OntologyContext {
                    version: ontology.version().get(),
                    definition: OntologyDefinition::from_ontology(ontology),
                },
                selected_entities: selected
                    .iter()
                    .enumerate()
                    .map(|(index, entity)| known_entity(format!("S{}", index + 1), entity, &limits))
                    .collect(),
                conversation: history
                    .iter()
                    .map(|turn| ConversationTurnContext {
                        question: turn.question.clone(),
                        entities: turn
                            .entity_ids
                            .iter()
                            .filter_map(|id| prepared.known_ref(*id))
                            .collect(),
                    })
                    .collect(),
                earlier_entities: earlier
                    .iter()
                    .enumerate()
                    .map(|(index, entity)| known_entity(format!("H{}", index + 1), entity, &limits))
                    .collect(),
                limits: InterpretationLimits {
                    max_traversal_depth: limits.max_traversal_depth,
                    max_path_depth: limits.max_path_depth,
                },
            })
            .await
            .map_err(|error| ai_failure(&error, "interpret"))?;

        let selected_ids: Vec<EntityId> = selected.iter().map(Entity::id).collect();
        let earlier_ids: Vec<EntityId> = earlier.iter().map(Entity::id).collect();
        let validated = validate_intent(
            &interpretation.intent,
            &PlanContext {
                ontology,
                selected: &selected_ids,
                earlier: &earlier_ids,
                limits: &limits,
            },
        )
        .map_err(plan_failure)?;
        let plan_summary = validated.summary();
        let operation = validated.plan.operation();
        let mut metadata = AnswerMetadata {
            interpret_prompt_version: interpretation.metadata.prompt_version.clone(),
            synthesize_prompt_version: None,
            provider: interpretation.metadata.provider.clone(),
            model: interpretation.metadata.model.clone(),
            conversation_turns: history.len(),
            duration_ms: 0,
        };
        // MindGraph's own caveats, on every answer of this question.
        let mut notices = Vec::new();
        if *missing > 0 {
            notices.push(format!(
                "{missing} {} from earlier questions {} not in this graph any more and {} ignored.",
                if *missing == 1 { "entity" } else { "entities" },
                if *missing == 1 { "is" } else { "are" },
                if *missing == 1 { "was" } else { "were" },
            ));
        }

        let finish = |answer: GraphRagAnswer| {
            info!(
                graph_id = %graph_id,
                request_id = %question.correlation_id,
                scope,
                operation = operation.as_str(),
                status = answer.status.as_str(),
                selected_count = selected.len(),
                conversation_turns = history.len(),
                earlier_entity_count = earlier.len(),
                entity_count = answer.evidence.entities().len(),
                relationship_count = answer.evidence.relationships().len(),
                traversal_operations = answer.retrieval.operations,
                candidates_inspected = answer.retrieval.candidates_inspected,
                context_bytes = answer.retrieval.context_bytes,
                retrieval_truncated = answer.retrieval.truncated,
                citation_count = answer.citations.len(),
                duration_ms = answer.metadata.duration_ms,
                "graphrag question answered"
            );
            answer
        };

        if let RetrievalPlan::Unsupported { .. } = validated.plan {
            metadata.duration_ms = elapsed_ms(started);
            return Ok(finish(GraphRagAnswer {
                status: AnswerStatus::InsufficientEvidence,
                answer: "This question does not map to a graph operation MindGraph supports (entity lookup, neighbours, shortest path or shared neighbour), so no graph evidence was retrieved. Try asking about specific entities or how they are connected.".to_owned(),
                source: AnswerSource::Unsupported,
                evidence: EvidenceGraph::default(),
                citations: Vec::new(),
                warnings: notices,
                retrieval: RetrievalReport {
                    scope,
                    plan: plan_summary,
                    operations: 0,
                    candidates_inspected: 0,
                    truncated: false,
                    context_bytes: 0,
                    limitations: validated.notes,
                },
                metadata,
            }));
        }

        let outcome = retrieve(
            self.explorer.graph_store(),
            graph_id,
            &validated.plan,
            &limits,
        )
        .await
        .map_err(retrieval_failure)?;
        let mut limitations = validated.notes.clone();
        limitations.extend(outcome.limitations.iter().cloned());
        let context = build_context(
            &outcome.evidence,
            &ContextMeta {
                scope,
                operation,
                complete: !outcome.stats.truncated,
                candidates_inspected: outcome.stats.candidates_inspected,
                limitations: &limitations,
            },
            &limits,
        );
        if context.reduced {
            limitations.push(
                "The evidence was reduced to fit the model context; some retrieved items are not shown."
                    .to_owned(),
            );
        }
        let truncated = outcome.stats.truncated || context.reduced;
        let report = |context_bytes: usize, limitations: Vec<String>| RetrievalReport {
            scope,
            plan: plan_summary.clone(),
            operations: outcome.stats.operations,
            candidates_inspected: outcome.stats.candidates_inspected,
            truncated,
            context_bytes,
            limitations,
        };

        if context.evidence.is_empty() {
            metadata.duration_ms = elapsed_ms(started);
            return Ok(finish(GraphRagAnswer {
                status: AnswerStatus::InsufficientEvidence,
                answer: "No matching graph evidence was found in the retrieved scope, so there is insufficient evidence to answer this question.".to_owned(),
                source: AnswerSource::NoEvidence,
                evidence: EvidenceGraph::default(),
                citations: Vec::new(),
                warnings: notices,
                retrieval: report(0, limitations),
                metadata,
            }));
        }

        let synthesis = self
            .ai
            .synthesize(&SynthesisRequest {
                correlation_id: question.correlation_id.clone(),
                question: text.clone(),
                earlier_questions: history.iter().map(|turn| turn.question.clone()).collect(),
                context: context.value.clone(),
                retrieval: RetrievalCompleteness {
                    complete: !truncated,
                    truncated,
                    limitations: limitations.clone(),
                },
            })
            .await
            .map_err(|error| ai_failure(&error, "synthesize"))?;
        metadata.synthesize_prompt_version = Some(synthesis.metadata.prompt_version.clone());
        metadata.provider = synthesis.metadata.provider.clone();
        metadata.model = synthesis.metadata.model.clone();

        let evidence = context.evidence;
        let (cleaned, inline, removed_inline) =
            clean_inline_citations(&synthesis.answer.answer, &evidence);
        let mut cited = synthesis.answer.evidence_refs.clone();
        cited.extend(inline);
        let check = evidence.resolve_citations(&cited);
        let mut warnings: Vec<String> = synthesis
            .answer
            .warnings
            .iter()
            .map(|warning| bounded(warning, MAX_WARNING_CHARS))
            .filter(|warning| !warning.is_empty())
            .take(MAX_WARNINGS)
            .collect();
        warnings.extend(notices);
        let unknown = check.unknown + removed_inline;
        if unknown > 0 {
            warnings.push(format!(
                "Removed {unknown} citation(s) that do not refer to the retrieved evidence."
            ));
        }
        let mut status = if outcome.resolution != Resolution::Resolved
            || synthesis.answer.insufficient_evidence
        {
            AnswerStatus::InsufficientEvidence
        } else if check.citations.is_empty() {
            warnings.push("The answer did not cite any retrieved evidence.".to_owned());
            AnswerStatus::InsufficientEvidence
        } else if truncated {
            AnswerStatus::Partial
        } else {
            AnswerStatus::Answered
        };
        let mut answer = bounded(&cleaned, MAX_ANSWER_CHARS);
        if answer.is_empty() {
            answer = "The model returned an empty answer.".to_owned();
            status = AnswerStatus::InsufficientEvidence;
        }
        metadata.duration_ms = elapsed_ms(started);
        Ok(finish(GraphRagAnswer {
            status,
            answer,
            source: AnswerSource::Model,
            citations: check.citations,
            warnings,
            retrieval: report(context.bytes, limitations),
            evidence,
            metadata,
        }))
    }
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::{Properties, Relationship, RelationshipId, TypeName};
    use mindgraph_graphrag::{EvidenceBuilder, EvidencePath};

    use super::*;

    fn evidence() -> EvidenceGraph {
        let graph_id = GraphId::new();
        let entity = |kind: &str| {
            Entity::new(
                EntityId::new(),
                graph_id,
                TypeName::new(kind).unwrap(),
                Properties::new(),
            )
        };
        let (a, b) = (entity("Customer"), entity("Device"));
        let link = Relationship::new(
            RelationshipId::new(),
            graph_id,
            TypeName::new("USES").unwrap(),
            a.id(),
            b.id(),
            Properties::new(),
        );
        let mut builder = EvidenceBuilder::new(graph_id, &GraphRagLimits::default());
        builder.add_all(&[&a, &b], &[&link]).unwrap();
        builder
            .add_path(EvidencePath::new(vec![a.id(), b.id()], vec![link.id()]))
            .unwrap();
        builder.finish()
    }

    #[test]
    fn inline_citations_keep_only_retrieved_references() {
        let evidence = evidence();
        let (text, cited, removed) = clean_inline_citations(
            "Alice [E1] uses the device [E2, R1, R7] via [P1]; see also [E9] and [note].",
            &evidence,
        );
        assert_eq!(
            text,
            "Alice [E1] uses the device [E2, R1] via [P1]; see also and [note]."
        );
        assert_eq!(cited, ["E1", "E2", "R1", "P1"]);
        assert_eq!(removed, 2);
    }

    #[test]
    fn follow_ups_keep_the_newest_earlier_entities() {
        let ids: Vec<EntityId> = (0..14).map(|_| EntityId::new()).collect();
        let turns = [
            ConversationTurn {
                question: "first".into(),
                entity_ids: ids[..8].to_vec(),
            },
            ConversationTurn {
                question: "second".into(),
                entity_ids: ids[6..12].to_vec(),
            },
        ];
        // The selected entity keeps its `S` reference and is not repeated.
        let kept = earlier_candidates(&turns, &[ids[11]]);
        assert_eq!(kept.len(), MAX_EARLIER_ENTITIES);
        // Newest question first (6..=10 of the second, the selected 11 is
        // skipped), then the first question's most relevant ids until the
        // bound: 5 is dropped. Numbered in conversation order.
        assert_eq!(kept, [&ids[..5], &ids[6..11]].concat());
    }

    #[test]
    fn questions_and_scopes_are_bounded() {
        assert!(check_question("  ").is_err());
        assert!(check_question(&"x".repeat(MAX_QUESTION_CHARS + 1)).is_err());
        assert_eq!(check_question(" Who? ").unwrap(), "Who?");
        assert!(check_scope(&AskScope::SelectedEntities(vec![])).is_err());
        let ids: Vec<EntityId> = (0..=MAX_SEED_ENTITIES).map(|_| EntityId::new()).collect();
        assert!(check_scope(&AskScope::SelectedEntities(ids)).is_err());
        let id = EntityId::new();
        assert_eq!(
            check_scope(&AskScope::SelectedEntities(vec![id, id])).unwrap(),
            [id]
        );
        assert!(check_scope(&AskScope::BoundedGraph).unwrap().is_empty());
    }
}
