//! The entity resolution persistence port (M9, ADR-009).
//!
//! A separate control-plane port next to [`MetadataStore`](crate::MetadataStore):
//! scans, duplicate candidates, review decisions and logical identity
//! groups. Implementations: [`MemoryEntityResolutionStore`](crate::MemoryEntityResolutionStore)
//! (the reference behaviour) and `PostgresEntityResolutionStore` in
//! `mindgraph-metadata`. A shared contract suite holds both to the same
//! behaviour.
//!
//! Nothing here stores property values: candidates carry entity ids, a score
//! and value-free evidence, and the graph itself stays in the GraphStore,
//! untouched by every decision.
//!
//! The decision rules that every implementation must apply inside its
//! atomic section live here as plain functions ([`check_repeat`],
//! [`plan_same_entity`], [`decision_record`]), so the backends cannot drift
//! apart.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mindgraph_domain::{
    EntityId, GraphId, OntologyVersion, ResolutionCandidateId, ResolutionDecisionId,
    ResolutionScanId, TypeName,
};
use mindgraph_entity_resolution::{
    IdentityGroup, MatchSignal, MergeError, MergePlan, RuleSet, ScanWarning, plan_merge,
};

use crate::MetadataStoreError;

/// The actor recorded for decisions in the single-user local version.
pub const LOCAL_ACTOR: &str = "local-user";

/// Pairs a scan found but did not store as new pending candidates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkippedPairs {
    /// The pair is already pending review from an earlier scan.
    pub already_pending: u32,
    /// The pair was rejected by an earlier review; it is not reopened
    /// automatically (see [`DecisionCommand::reconsider`]).
    pub previously_rejected: u32,
    /// Both entities already belong to one identity group.
    pub already_linked: u32,
}

/// One bounded resolution scan and its coverage. Holds rule snapshots,
/// counts and value-free warnings, never entity properties.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolutionScan {
    /// The scan id.
    pub id: ResolutionScanId,
    /// The scanned graph.
    pub graph_id: GraphId,
    /// The scanned entity type.
    pub entity_type: TypeName,
    /// The validated rules, as used.
    pub rules: RuleSet,
    /// The active ontology version the rules were validated against.
    pub ontology_version: OntologyVersion,
    /// The requested entity bound.
    pub max_entities: u32,
    /// Entities examined (at most `max_entities`).
    pub entities_examined: u32,
    /// Whether the graph holds more entities of the type than were examined.
    pub truncated: bool,
    /// Distinct pairs compared.
    pub pairs_compared: u32,
    /// The comparison budget.
    pub comparison_budget: u32,
    /// Whether the budget stopped the scan early.
    pub comparison_budget_reached: bool,
    /// Blocks skipped for their size.
    pub oversized_blocks: u32,
    /// Whether lower-scoring candidates were dropped by the candidate cap.
    pub candidate_limit_reached: bool,
    /// New pending candidates stored by this scan. Set by the store.
    pub candidates_found: u32,
    /// Pairs found but not stored. Set by the store.
    pub skipped: SkippedPairs,
    /// Data-quality findings.
    pub warnings: Vec<ScanWarning>,
    /// Wall-clock duration of the scan in milliseconds.
    pub duration_ms: u64,
    /// When the scan ran.
    pub created_at: DateTime<Utc>,
}

impl ResolutionScan {
    /// Whether every entity of the type was examined and every candidate
    /// pair in every block was compared and kept.
    #[must_use]
    pub const fn exhaustive(&self) -> bool {
        !self.truncated
            && !self.comparison_budget_reached
            && self.oversized_blocks == 0
            && !self.candidate_limit_reached
    }
}

/// The review state of a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateStatus {
    /// Waiting for a reviewer.
    Pending,
    /// Reviewed as the same real-world entity.
    Accepted,
    /// Reviewed as different entities.
    Rejected,
}

impl CandidateStatus {
    /// The stable name of this status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    /// Parses a status by its stable name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "pending" => Some(Self::Pending),
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// A candidate to store with a scan.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCandidate {
    /// The candidate id.
    pub id: ResolutionCandidateId,
    /// The smaller entity id.
    pub entity_a: EntityId,
    /// The larger entity id.
    pub entity_b: EntityId,
    /// The ranking score.
    pub score: f64,
    /// Value-free evidence, one signal per rule.
    pub signals: Vec<MatchSignal>,
}

/// A stored duplicate candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolutionCandidate {
    /// The candidate id.
    pub id: ResolutionCandidateId,
    /// The scan that found it.
    pub scan_id: ResolutionScanId,
    /// Its graph.
    pub graph_id: GraphId,
    /// The entity type of both entities.
    pub entity_type: TypeName,
    /// The smaller entity id.
    pub entity_a: EntityId,
    /// The larger entity id.
    pub entity_b: EntityId,
    /// The ranking score (a heuristic, not a probability).
    pub score: f64,
    /// Value-free evidence, one signal per rule of the scan.
    pub signals: Vec<MatchSignal>,
    /// The review state.
    pub status: CandidateStatus,
    /// When it was found.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// When it was last decided.
    pub decided_at: Option<DateTime<Utc>>,
}

impl ResolutionCandidate {
    /// `kind:property` labels of the agreeing signals, for provenance.
    #[must_use]
    pub fn matched_rules(&self) -> Vec<String> {
        self.signals
            .iter()
            .filter(|signal| signal.matched)
            .map(|signal| format!("{}:{}", signal.rule.as_str(), signal.property))
            .collect()
    }
}

/// Which candidates to list. Ordered by score (highest first), then id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandidateQuery {
    /// Only candidates in this state.
    pub status: Option<CandidateStatus>,
    /// Only candidates of this scan.
    pub scan_id: Option<ResolutionScanId>,
    /// Only candidates involving this entity.
    pub entity_id: Option<EntityId>,
    /// At most this many.
    pub limit: usize,
    /// Continue after this candidate (keyset pagination).
    pub after: Option<ResolutionCandidateId>,
}

/// Candidates per state, under a query's scan and entity filters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CandidateCounts {
    /// Pending candidates.
    pub pending: u64,
    /// Accepted candidates.
    pub accepted: u64,
    /// Rejected candidates.
    pub rejected: u64,
}

/// One page of candidates.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidatePage {
    /// The candidates of this page.
    pub candidates: Vec<ResolutionCandidate>,
    /// The cursor of the next page, if there is one.
    pub next: Option<ResolutionCandidateId>,
    /// Counts per state.
    pub counts: CandidateCounts,
}

/// A reviewer's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecisionKind {
    /// Both entities represent the same real-world entity.
    SameEntity,
    /// The entities are different.
    DifferentEntities,
}

impl DecisionKind {
    /// The stable name of this decision.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SameEntity => "same_entity",
            Self::DifferentEntities => "different_entities",
        }
    }

    /// Parses a decision by its stable name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "same_entity" => Some(Self::SameEntity),
            "different_entities" => Some(Self::DifferentEntities),
            _ => None,
        }
    }
}

/// An immutable review decision with its provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolutionDecision {
    /// The decision id.
    pub id: ResolutionDecisionId,
    /// Its graph.
    pub graph_id: GraphId,
    /// The reviewed candidate.
    pub candidate_id: ResolutionCandidateId,
    /// The scan that found the candidate.
    pub scan_id: ResolutionScanId,
    /// The decision.
    pub kind: DecisionKind,
    /// The candidate's smaller entity id.
    pub entity_a: EntityId,
    /// The candidate's larger entity id.
    pub entity_b: EntityId,
    /// `same_entity`: the canonical entity the reviewer chose.
    pub canonical_entity_id: Option<EntityId>,
    /// `same_entity`: the canonical entities of the groups before.
    pub previous_canonical_ids: Vec<EntityId>,
    /// `same_entity`: members whose mapping this decision wrote.
    pub linked_entity_ids: Vec<EntityId>,
    /// The candidate's score at review time.
    pub score: f64,
    /// The agreeing rules (`kind:property`).
    pub matched_rules: Vec<String>,
    /// Who decided ([`LOCAL_ACTOR`] in the local version).
    pub actor: String,
    /// Whether this decision reconsidered an earlier rejection.
    pub reconsidered: bool,
    /// When it was decided.
    pub created_at: DateTime<Utc>,
}

/// A decision to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionCommand {
    /// The new decision's id.
    pub id: ResolutionDecisionId,
    /// The graph.
    pub graph_id: GraphId,
    /// The reviewed candidate.
    pub candidate_id: ResolutionCandidateId,
    /// The decision.
    pub kind: DecisionKind,
    /// `same_entity`: the canonical entity (a member of the approved group).
    pub canonical: Option<EntityId>,
    /// Explicitly review a rejected candidate again.
    pub reconsider: bool,
    /// Who decides.
    pub actor: String,
    /// When.
    pub decided_at: DateTime<Utc>,
    /// The largest identity group allowed.
    pub max_group_members: usize,
}

/// One member's mapping to its canonical entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityMembership {
    /// The member.
    pub entity_id: EntityId,
    /// The decision that last wrote this mapping.
    pub decision_id: ResolutionDecisionId,
    /// When it was written.
    pub linked_at: DateTime<Utc>,
}

/// A stored logical identity group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIdentityGroup {
    /// The group.
    pub group: IdentityGroup,
    /// The entity type of every member.
    pub entity_type: TypeName,
    /// Every member's mapping, in entity id order.
    pub members: Vec<IdentityMembership>,
}

/// The result of recording a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionOutcome {
    /// The decision (the earlier one for a replay).
    pub decision: ResolutionDecision,
    /// The candidate after the decision.
    pub candidate: ResolutionCandidate,
    /// The identity group of the candidate's entities, if any.
    pub group: Option<StoredIdentityGroup>,
    /// Whether an identical earlier decision was returned instead of a new
    /// one (idempotent repeat).
    pub replayed: bool,
}

/// An error from an [`EntityResolutionStore`].
#[derive(Debug, thiserror::Error)]
pub enum ResolutionStoreError {
    /// The candidate does not exist in the graph.
    #[error("candidate {0} was not found")]
    CandidateNotFound(ResolutionCandidateId),

    /// The pagination cursor is not a candidate of the graph.
    #[error("unknown cursor")]
    InvalidCursor,

    /// The decision contradicts the candidate's earlier decision.
    #[error("{0}")]
    DecisionConflict(String),

    /// The decision would break identity group rules.
    #[error("{0}")]
    IdentityConflict(String),

    /// The input breaks a store invariant (a bug in the caller).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The backend failed.
    #[error(transparent)]
    Store(#[from] MetadataStoreError),
}

/// Persistence of resolution scans, candidates, decisions and identity
/// groups. Object safe; share it as `Arc<dyn EntityResolutionStore>`.
///
/// # Contract
///
/// - **Graph isolation.** Every read and write is scoped to one graph; a
///   record of another graph is missing.
/// - **Atomic scans.** A scan and its candidates are stored together or not
///   at all. Pairs already pending, previously rejected or already linked
///   are skipped (and counted), all under the same lock as decisions, so
///   concurrent scans and decisions cannot create a second pending
///   candidate for one pair.
/// - **Atomic decisions.** A decision record, the candidate's new state and
///   every identity mapping it writes are stored together, serialised per
///   graph so concurrent approvals cannot produce conflicting canonical
///   mappings. Groups stay flat: every member maps to a canonical entity
///   that maps to itself, with one entity type per group.
/// - **Append-only provenance.** Decisions are never updated or deleted.
#[async_trait]
pub trait EntityResolutionStore: Send + Sync {
    /// Stores `scan` with `candidates`, skipping pairs that are already
    /// pending, previously rejected or already linked. Returns the stored
    /// scan with `candidates_found` and `skipped` set.
    ///
    /// # Errors
    ///
    /// [`ResolutionStoreError::InvalidInput`] for an invalid candidate (a
    /// self-pair, unordered or repeated pair); nothing is stored then.
    async fn create_scan_with_candidates(
        &self,
        scan: ResolutionScan,
        candidates: Vec<NewCandidate>,
    ) -> Result<ResolutionScan, ResolutionStoreError>;

    /// One scan of the graph.
    async fn get_scan(
        &self,
        graph_id: GraphId,
        scan_id: ResolutionScanId,
    ) -> Result<Option<ResolutionScan>, ResolutionStoreError>;

    /// The graph's most recent scans, newest first.
    async fn list_scans(
        &self,
        graph_id: GraphId,
        limit: usize,
    ) -> Result<Vec<ResolutionScan>, ResolutionStoreError>;

    /// One page of candidates.
    ///
    /// # Errors
    ///
    /// [`ResolutionStoreError::InvalidCursor`] when `after` is not a
    /// candidate of the graph.
    async fn list_candidates(
        &self,
        graph_id: GraphId,
        query: &CandidateQuery,
    ) -> Result<CandidatePage, ResolutionStoreError>;

    /// One candidate of the graph.
    async fn get_candidate(
        &self,
        graph_id: GraphId,
        candidate_id: ResolutionCandidateId,
    ) -> Result<Option<ResolutionCandidate>, ResolutionStoreError>;

    /// A candidate's decisions, oldest first.
    async fn list_decisions(
        &self,
        graph_id: GraphId,
        candidate_id: ResolutionCandidateId,
    ) -> Result<Vec<ResolutionDecision>, ResolutionStoreError>;

    /// Records a decision atomically (see the trait docs and
    /// [`check_repeat`]).
    ///
    /// # Errors
    ///
    /// [`ResolutionStoreError::CandidateNotFound`],
    /// [`ResolutionStoreError::DecisionConflict`] or
    /// [`ResolutionStoreError::IdentityConflict`].
    async fn record_decision(
        &self,
        command: DecisionCommand,
    ) -> Result<DecisionOutcome, ResolutionStoreError>;

    /// The identity group of `entity_id`, or `None` when it is in none.
    async fn get_identity_group(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<Option<StoredIdentityGroup>, ResolutionStoreError>;

    /// The canonical entity of `entity_id`: itself when it is in no group.
    async fn resolve_canonical_identity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<EntityId, ResolutionStoreError> {
        Ok(self
            .get_identity_group(graph_id, entity_id)
            .await?
            .map_or(entity_id, |stored| stored.group.canonical()))
    }
}

// --- decision rules shared by every implementation -----------------------------------------

/// What to do with a decision on a candidate that may already be decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Repeat {
    /// Record the decision; `reconsidered` when it reviews a rejection again.
    Proceed {
        /// Whether an earlier rejection is being reconsidered.
        reconsidered: bool,
    },
    /// The same decision was already recorded: return it (idempotent).
    Replay(Box<ResolutionDecision>),
}

/// Applies the repeat rules to a candidate and its latest decision:
///
/// - a pending candidate takes any decision;
/// - repeating the latest decision (same kind, same canonical entity)
///   replays it;
/// - a rejected candidate can be accepted only with `reconsider`;
/// - any other change of an earlier decision is a conflict: accepted
///   mappings cannot be undone in v0.1, and a decision is never silently
///   overwritten.
///
/// # Errors
///
/// [`ResolutionStoreError::DecisionConflict`].
pub fn check_repeat(
    candidate: &ResolutionCandidate,
    latest: Option<&ResolutionDecision>,
    command: &DecisionCommand,
) -> Result<Repeat, ResolutionStoreError> {
    let Some(latest) = latest.filter(|_| candidate.status != CandidateStatus::Pending) else {
        return Ok(Repeat::Proceed {
            reconsidered: false,
        });
    };
    let same = latest.kind == command.kind
        && (command.kind == DecisionKind::DifferentEntities
            || latest.canonical_entity_id == command.canonical);
    if same {
        return Ok(Repeat::Replay(Box::new(latest.clone())));
    }
    match (candidate.status, command.kind) {
        (CandidateStatus::Rejected, DecisionKind::SameEntity) if command.reconsider => {
            Ok(Repeat::Proceed { reconsidered: true })
        }
        (CandidateStatus::Rejected, DecisionKind::SameEntity) => {
            Err(ResolutionStoreError::DecisionConflict(
                "this candidate was already reviewed as different entities; set reconsider to review it again"
                    .into(),
            ))
        }
        (CandidateStatus::Accepted, _) => Err(ResolutionStoreError::DecisionConflict(
            "this candidate was already accepted with another decision; accepted identity mappings cannot be changed in v0.1"
                .into(),
        )),
        _ => Err(ResolutionStoreError::DecisionConflict(
            "this candidate already has a different decision".into(),
        )),
    }
}

/// Plans a `same_entity` decision from the current groups of the candidate's
/// entities, mapping planning errors to conflicts.
///
/// # Errors
///
/// [`ResolutionStoreError::IdentityConflict`] (canonical outside the group,
/// group too large) or [`ResolutionStoreError::InvalidInput`].
pub fn plan_same_entity(
    candidate: &ResolutionCandidate,
    group_a: Option<&IdentityGroup>,
    group_b: Option<&IdentityGroup>,
    command: &DecisionCommand,
) -> Result<MergePlan, ResolutionStoreError> {
    let canonical = command.canonical.ok_or_else(|| {
        ResolutionStoreError::InvalidInput("same_entity needs a canonical entity".into())
    })?;
    plan_merge(
        candidate.entity_a,
        candidate.entity_b,
        group_a,
        group_b,
        canonical,
        command.max_group_members,
    )
    .map_err(|error| match error {
        MergeError::CanonicalOutsideGroup | MergeError::GroupTooLarge { .. } => {
            ResolutionStoreError::IdentityConflict(error.to_string())
        }
        MergeError::SelfPair | MergeError::InconsistentGroup => {
            ResolutionStoreError::InvalidInput(error.to_string())
        }
    })
}

/// The members on each side of a `same_entity` decision whose earlier
/// rejections would be contradicted: `None` when both entities are already
/// in one group (nothing new is linked).
#[must_use]
pub fn linked_sides(
    candidate: &ResolutionCandidate,
    group_a: Option<&IdentityGroup>,
    group_b: Option<&IdentityGroup>,
) -> Option<(Vec<EntityId>, Vec<EntityId>)> {
    let side = |entity: EntityId, group: Option<&IdentityGroup>| -> Vec<EntityId> {
        group.map_or_else(|| vec![entity], |g| g.members().iter().copied().collect())
    };
    let (a, b) = (
        side(candidate.entity_a, group_a),
        side(candidate.entity_b, group_b),
    );
    (a != b).then_some((a, b))
}

/// The conflict reported when linking would contradict a rejection.
#[must_use]
pub fn rejected_pair_conflict() -> ResolutionStoreError {
    ResolutionStoreError::IdentityConflict(
        "linking these entities would contradict an earlier review that marked two of the group's entities as different; reconsider that candidate first"
            .into(),
    )
}

/// The conflict reported when rejecting entities that are already linked.
#[must_use]
pub fn linked_pair_conflict() -> ResolutionStoreError {
    ResolutionStoreError::IdentityConflict(
        "these entities already belong to one identity group; splitting identity groups is not supported in v0.1"
            .into(),
    )
}

/// The decision record for `command` on `candidate`.
#[must_use]
pub fn decision_record(
    candidate: &ResolutionCandidate,
    command: &DecisionCommand,
    plan: Option<&MergePlan>,
    reconsidered: bool,
) -> ResolutionDecision {
    ResolutionDecision {
        id: command.id,
        graph_id: candidate.graph_id,
        candidate_id: candidate.id,
        scan_id: candidate.scan_id,
        kind: command.kind,
        entity_a: candidate.entity_a,
        entity_b: candidate.entity_b,
        canonical_entity_id: plan.map(|plan| plan.canonical),
        previous_canonical_ids: plan
            .map(|plan| plan.previous_canonicals.clone())
            .unwrap_or_default(),
        linked_entity_ids: plan.map(|plan| plan.changed.clone()).unwrap_or_default(),
        score: candidate.score,
        matched_rules: candidate.matched_rules(),
        actor: command.actor.clone(),
        reconsidered,
        created_at: command.decided_at,
    }
}

/// Checks the candidates of a scan before storing: ordered, distinct,
/// non-self pairs with a score in `[0, 1]`.
///
/// # Errors
///
/// [`ResolutionStoreError::InvalidInput`].
pub fn check_new_candidates(candidates: &[NewCandidate]) -> Result<(), ResolutionStoreError> {
    let mut pairs = std::collections::HashSet::new();
    for candidate in candidates {
        if candidate.entity_a >= candidate.entity_b {
            return Err(ResolutionStoreError::InvalidInput(
                "a candidate pair must name two different entities, smaller id first".into(),
            ));
        }
        if !pairs.insert((candidate.entity_a, candidate.entity_b)) {
            return Err(ResolutionStoreError::InvalidInput(
                "a scan cannot contain the same pair twice".into(),
            ));
        }
        if !(0.0..=1.0).contains(&candidate.score) {
            return Err(ResolutionStoreError::InvalidInput(
                "a candidate score must be between 0 and 1".into(),
            ));
        }
    }
    Ok(())
}
