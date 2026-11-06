//! Entity resolution use cases (M9, ADR-009): bounded duplicate candidate
//! scans, human review and logical identity groups.
//!
//! ```text
//! mindgraph-api ─► EntityResolutionService ─┬► GraphExplorerService (graph check, ontology,
//!                                            │                       bounded GraphStore reads)
//!                                            ├► mindgraph-entity-resolution (pure matching)
//!                                            └► EntityResolutionStore (PostgreSQL | memory)
//! ```
//!
//! **Logical, never physical.** Approving a candidate records metadata: a
//! decision with its provenance and a canonical identity mapping. It never
//! writes to the GraphStore: no entity is deleted, no relationship is
//! rewired and no property is overwritten. Graph Explorer, GraphRAG and MCP
//! keep returning the real entities and relationships.
//!
//! **Bounded, and honest about it.** A scan reads at most `max_entities`
//! entities of one type (the first in id order, at most 999) through the
//! GraphStore's bounded search, compares at most 20,000 pairs within a 30 s
//! deadline, and reports every limit it hit. Graph data is read and matched
//! before the metadata transaction starts.
//!
//! Logs carry ids, counts and durations, never names, emails, phones or
//! other property values.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{SubsecRound, Utc};
use mindgraph_domain::{
    Entity, EntityId, GraphId, ResolutionCandidateId, ResolutionDecisionId, ResolutionScanId,
    TypeName,
};
use mindgraph_entity_resolution::{RuleError, RuleSet, RuleSpec, ScanLimits, find_candidates};
use mindgraph_graph::EntitySearch;
use tokio::sync::Semaphore;
use tracing::{info, warn};

use crate::explorer::graph_failure;
use crate::resolution_store::{
    CandidatePage, CandidateQuery, DecisionCommand, DecisionKind, DecisionOutcome,
    EntityResolutionStore, IdentityMembership, LOCAL_ACTOR, NewCandidate, ResolutionCandidate,
    ResolutionDecision, ResolutionScan, ResolutionStoreError, SkippedPairs,
};
use crate::{ApplicationError, GraphExplorerService};

/// Entities examined when a scan names no bound.
pub const DEFAULT_SCAN_ENTITIES: usize = 400;
/// Smallest entity bound.
pub const MIN_SCAN_ENTITIES: usize = 2;
/// Largest entity bound: the GraphStore search returns at most 1,000
/// entities, and one more than the bound is read to detect truncation.
pub const MAX_SCAN_ENTITIES: usize = EntitySearch::MAX_LIMIT - 1;
/// Candidates per page when none is requested.
pub const DEFAULT_CANDIDATE_PAGE: usize = 20;
/// Largest candidate page.
pub const MAX_CANDIDATE_PAGE: usize = 100;
/// Largest scan history page.
pub const MAX_SCAN_HISTORY: usize = 50;
/// Default deadline of one scan, waiting for a slot included.
pub const DEFAULT_SCAN_TIMEOUT: Duration = Duration::from_secs(30);
/// Default scans running at the same time.
pub const DEFAULT_SCAN_CONCURRENCY: usize = 2;

/// An entity resolution failure a client can act on.
#[derive(Debug, thiserror::Error)]
pub enum ResolutionError {
    /// The entity type is not in the graph's active ontology.
    #[error("entity type `{0}` is not in the graph's ontology")]
    UnknownEntityType(String),

    /// A matching rule cannot be used.
    #[error(transparent)]
    InvalidRule(#[from] RuleError),

    /// The scan does not exist in the graph.
    #[error("resolution scan {0} was not found")]
    ScanNotFound(ResolutionScanId),

    /// The candidate does not exist in the graph.
    #[error("candidate {0} was not found")]
    CandidateNotFound(ResolutionCandidateId),

    /// The decision request is malformed.
    #[error("{0}")]
    InvalidDecision(String),

    /// The decision contradicts the candidate's earlier decision.
    #[error("{0}")]
    DecisionConflict(String),

    /// The decision would break identity group rules.
    #[error("{0}")]
    IdentityConflict(String),

    /// An entity of the candidate is no longer in the graph.
    #[error("entity {0} of this candidate is no longer in the graph")]
    EntityUnavailable(EntityId),

    /// The candidate pagination cursor is unknown.
    #[error("unknown candidate cursor")]
    InvalidCursor,

    /// The scan did not finish within its deadline.
    #[error("the resolution scan did not finish within {0} seconds")]
    ScanTimeout(u64),
}

impl From<ResolutionError> for ApplicationError {
    fn from(error: ResolutionError) -> Self {
        Self::Resolution(error)
    }
}

fn store_failure(error: ResolutionStoreError) -> ApplicationError {
    match error {
        ResolutionStoreError::CandidateNotFound(id) => {
            ResolutionError::CandidateNotFound(id).into()
        }
        ResolutionStoreError::InvalidCursor => ResolutionError::InvalidCursor.into(),
        ResolutionStoreError::DecisionConflict(message) => {
            ResolutionError::DecisionConflict(message).into()
        }
        ResolutionStoreError::IdentityConflict(message) => {
            ResolutionError::IdentityConflict(message).into()
        }
        ResolutionStoreError::InvalidInput(message) => ApplicationError::internal(message),
        ResolutionStoreError::Store(error) => error.into(),
    }
}

/// Settings of the entity resolution service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityResolutionConfig {
    /// Deadline of one scan, waiting for a slot included.
    pub scan_timeout: Duration,
    /// Scans running at the same time.
    pub concurrency: usize,
    /// Comparison, block and candidate bounds of one scan.
    pub limits: ScanLimits,
    /// Largest identity group.
    pub max_group_members: usize,
}

impl Default for EntityResolutionConfig {
    fn default() -> Self {
        Self {
            scan_timeout: DEFAULT_SCAN_TIMEOUT,
            concurrency: DEFAULT_SCAN_CONCURRENCY,
            limits: ScanLimits::default(),
            max_group_members: mindgraph_entity_resolution::DEFAULT_MAX_GROUP_MEMBERS,
        }
    }
}

/// A scan as a client requests it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanRequest {
    /// The entity type to scan.
    pub entity_type: String,
    /// Entities to examine (`2..=999`, default 400).
    pub max_entities: Option<usize>,
    /// The matching rules.
    pub rules: Vec<RuleSpec>,
}

/// A decision as a client requests it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionRequest {
    /// The decision.
    pub kind: DecisionKind,
    /// `same_entity`: the canonical entity.
    pub canonical_entity_id: Option<EntityId>,
    /// Review a rejected candidate again.
    pub reconsider: bool,
}

/// The logical identity of one entity. `physical_graph_merged` is always
/// `false`: the graph keeps every original entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityView {
    /// The entity asked about.
    pub entity_id: EntityId,
    /// Its canonical entity (itself when it is in no group).
    pub canonical_entity_id: EntityId,
    /// The group's members with provenance; empty when in no group.
    pub members: Vec<IdentityMembership>,
    /// The entity type of the group, when in one.
    pub entity_type: Option<TypeName>,
}

impl IdentityView {
    fn of(entity_id: EntityId, stored: Option<crate::StoredIdentityGroup>) -> Self {
        match stored {
            Some(stored) => Self {
                entity_id,
                canonical_entity_id: stored.group.canonical(),
                members: stored.members,
                entity_type: Some(stored.entity_type),
            },
            None => Self {
                entity_id,
                canonical_entity_id: entity_id,
                members: Vec::new(),
                entity_type: None,
            },
        }
    }

    /// Whether the entity belongs to an identity group.
    #[must_use]
    pub fn resolved(&self) -> bool {
        !self.members.is_empty()
    }

    /// Whether the entity is its group's canonical entity (or in no group).
    #[must_use]
    pub fn is_canonical(&self) -> bool {
        self.entity_id == self.canonical_entity_id
    }

    /// The member ids (just the entity when in no group).
    #[must_use]
    pub fn member_ids(&self) -> Vec<EntityId> {
        if self.members.is_empty() {
            vec![self.entity_id]
        } else {
            self.members.iter().map(|m| m.entity_id).collect()
        }
    }
}

/// A candidate with both original entities, read from the GraphStore, and
/// its decision history.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateDetail {
    /// The candidate.
    pub candidate: ResolutionCandidate,
    /// Entity A as stored in the graph, if it still exists.
    pub entity_a: Option<Entity>,
    /// Entity B as stored in the graph, if it still exists.
    pub entity_b: Option<Entity>,
    /// Decisions about this candidate, oldest first.
    pub decisions: Vec<ResolutionDecision>,
    /// The current identity of entity A.
    pub identity_a: IdentityView,
    /// The current identity of entity B.
    pub identity_b: IdentityView,
}

/// The result of a decision, with the identity it produced.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionResult {
    /// The stored outcome.
    pub outcome: DecisionOutcome,
    /// The identity of the candidate's entity A after the decision.
    pub identity: IdentityView,
}

/// Bounded duplicate detection, review and logical identity.
#[derive(Clone)]
pub struct EntityResolutionService {
    explorer: GraphExplorerService,
    store: Arc<dyn EntityResolutionStore>,
    config: EntityResolutionConfig,
    permits: Arc<Semaphore>,
}

impl std::fmt::Debug for EntityResolutionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityResolutionService")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

impl EntityResolutionService {
    /// A service reading graphs through `explorer` (which owns workspace and
    /// graph isolation) and persisting through `store`.
    #[must_use]
    pub fn new(
        explorer: GraphExplorerService,
        store: Arc<dyn EntityResolutionStore>,
        config: EntityResolutionConfig,
    ) -> Self {
        let config = EntityResolutionConfig {
            scan_timeout: config
                .scan_timeout
                .clamp(Duration::from_secs(1), Duration::from_secs(300)),
            concurrency: config.concurrency.clamp(1, 16),
            limits: config.limits,
            max_group_members: config.max_group_members.max(2),
        };
        Self {
            explorer,
            store,
            permits: Arc::new(Semaphore::new(config.concurrency)),
            config,
        }
    }

    /// The effective settings.
    #[must_use]
    pub const fn config(&self) -> &EntityResolutionConfig {
        &self.config
    }

    /// Runs a bounded scan over one entity type and stores its candidates.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for `max_entities` out of range,
    /// [`ApplicationError::GraphNotFound`], [`ResolutionError::UnknownEntityType`],
    /// [`ResolutionError::InvalidRule`], [`ResolutionError::ScanTimeout`], or a
    /// backend failure.
    pub async fn scan(
        &self,
        graph_id: GraphId,
        request: ScanRequest,
    ) -> Result<ResolutionScan, ApplicationError> {
        let started = Instant::now();
        let max_entities = request.max_entities.unwrap_or(DEFAULT_SCAN_ENTITIES);
        if !(MIN_SCAN_ENTITIES..=MAX_SCAN_ENTITIES).contains(&max_entities) {
            return Err(ApplicationError::InvalidRequest(format!(
                "max_entities must be between {MIN_SCAN_ENTITIES} and {MAX_SCAN_ENTITIES}"
            )));
        }
        self.explorer.ensure_graph(graph_id).await?;
        let ontology = self.explorer.active_ontology(graph_id).await?;
        let requested = request.entity_type.trim();
        let entity_type = TypeName::new(requested)
            .ok()
            .and_then(|name| ontology.entity_type(name.as_str()))
            .ok_or_else(|| {
                let shown: String = requested.chars().take(64).collect();
                ResolutionError::UnknownEntityType(shown)
            })?;
        let rules =
            RuleSet::validate(entity_type, &request.rules).map_err(ResolutionError::from)?;
        let deadline = self.config.scan_timeout;
        let run = self.run_scan(graph_id, rules, max_entities, ontology.version(), started);
        match tokio::time::timeout(deadline, run).await {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    graph_id = %graph_id,
                    timeout_s = deadline.as_secs(),
                    "entity resolution scan timed out"
                );
                Err(ResolutionError::ScanTimeout(deadline.as_secs()).into())
            }
        }
    }

    async fn run_scan(
        &self,
        graph_id: GraphId,
        rules: RuleSet,
        max_entities: usize,
        ontology_version: mindgraph_domain::OntologyVersion,
        started: Instant,
    ) -> Result<ResolutionScan, ApplicationError> {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(ApplicationError::internal)?;
        // One more than the bound reveals whether the scan covers every
        // entity of the type. No metadata transaction is open here.
        let search = EntitySearch::new()
            .with_entity_type(rules.entity_type.clone())
            .with_limit(max_entities + 1)
            .map_err(graph_failure)?;
        let mut entities = self
            .explorer
            .graph_store()
            .search_entities(graph_id, &search)
            .await
            .map_err(graph_failure)?;
        let truncated = entities.len() > max_entities;
        entities.truncate(max_entities);

        let limits = self.config.limits;
        let task_rules = rules.clone();
        let report =
            tokio::task::spawn_blocking(move || find_candidates(&entities, &task_rules, &limits))
                .await
                .map_err(ApplicationError::internal)?;

        let scan = ResolutionScan {
            id: ResolutionScanId::new(),
            graph_id,
            entity_type: rules.entity_type.clone(),
            rules,
            ontology_version,
            max_entities: count(max_entities),
            entities_examined: count(report.entities_examined),
            truncated,
            pairs_compared: count(report.pairs_compared),
            comparison_budget: count(limits.max_comparisons),
            comparison_budget_reached: report.comparison_budget_reached,
            oversized_blocks: count(report.oversized_blocks),
            candidate_limit_reached: report.candidate_limit_reached,
            candidates_found: 0,
            skipped: SkippedPairs::default(),
            warnings: report.warnings,
            duration_ms: elapsed_ms(started),
            created_at: Utc::now().trunc_subsecs(6),
        };
        let candidates = report
            .candidates
            .into_iter()
            .map(|found| NewCandidate {
                id: ResolutionCandidateId::new(),
                entity_a: found.entity_a,
                entity_b: found.entity_b,
                score: found.score,
                signals: found.signals,
            })
            .collect();
        let scan = self
            .store
            .create_scan_with_candidates(scan, candidates)
            .await
            .map_err(store_failure)?;
        info!(
            graph_id = %graph_id,
            scan_id = %scan.id,
            entity_type = %scan.entity_type,
            rule_count = scan.rules.rules.len(),
            entities_examined = scan.entities_examined,
            pairs_compared = scan.pairs_compared,
            candidate_count = scan.candidates_found,
            truncated = scan.truncated,
            exhaustive = scan.exhaustive(),
            duration_ms = elapsed_ms(started),
            "entity resolution scan completed"
        );
        Ok(scan)
    }

    /// One scan of the graph.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ResolutionError::ScanNotFound`]
    /// (also for a scan of another graph), or a backend failure.
    pub async fn get_scan(
        &self,
        graph_id: GraphId,
        scan_id: ResolutionScanId,
    ) -> Result<ResolutionScan, ApplicationError> {
        self.explorer.ensure_graph(graph_id).await?;
        self.store
            .get_scan(graph_id, scan_id)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| ResolutionError::ScanNotFound(scan_id).into())
    }

    /// The graph's most recent scans, newest first.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for a limit out of range,
    /// [`ApplicationError::GraphNotFound`], or a backend failure.
    pub async fn list_scans(
        &self,
        graph_id: GraphId,
        limit: Option<usize>,
    ) -> Result<Vec<ResolutionScan>, ApplicationError> {
        let limit = limit.unwrap_or(10);
        if !(1..=MAX_SCAN_HISTORY).contains(&limit) {
            return Err(ApplicationError::InvalidRequest(format!(
                "limit must be between 1 and {MAX_SCAN_HISTORY}"
            )));
        }
        self.explorer.ensure_graph(graph_id).await?;
        self.store
            .list_scans(graph_id, limit)
            .await
            .map_err(store_failure)
    }

    /// One page of candidates, highest score first.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for a limit out of range,
    /// [`ApplicationError::GraphNotFound`], [`ResolutionError::InvalidCursor`],
    /// or a backend failure.
    pub async fn list_candidates(
        &self,
        graph_id: GraphId,
        query: CandidateQuery,
    ) -> Result<CandidatePage, ApplicationError> {
        if !(1..=MAX_CANDIDATE_PAGE).contains(&query.limit) {
            return Err(ApplicationError::InvalidRequest(format!(
                "limit must be between 1 and {MAX_CANDIDATE_PAGE}"
            )));
        }
        self.explorer.ensure_graph(graph_id).await?;
        self.store
            .list_candidates(graph_id, &query)
            .await
            .map_err(store_failure)
    }

    /// A candidate with both original entities and its decision history.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ResolutionError::CandidateNotFound`],
    /// or a backend failure.
    pub async fn get_candidate(
        &self,
        graph_id: GraphId,
        candidate_id: ResolutionCandidateId,
    ) -> Result<CandidateDetail, ApplicationError> {
        self.explorer.ensure_graph(graph_id).await?;
        let candidate = self
            .store
            .get_candidate(graph_id, candidate_id)
            .await
            .map_err(store_failure)?
            .ok_or(ResolutionError::CandidateNotFound(candidate_id))?;
        let graph = self.explorer.graph_store();
        let entity_a = graph
            .get_entity(graph_id, candidate.entity_a)
            .await
            .map_err(graph_failure)?;
        let entity_b = graph
            .get_entity(graph_id, candidate.entity_b)
            .await
            .map_err(graph_failure)?;
        let decisions = self
            .store
            .list_decisions(graph_id, candidate_id)
            .await
            .map_err(store_failure)?;
        let identity_a = IdentityView::of(
            candidate.entity_a,
            self.store
                .get_identity_group(graph_id, candidate.entity_a)
                .await
                .map_err(store_failure)?,
        );
        let identity_b = IdentityView::of(
            candidate.entity_b,
            self.store
                .get_identity_group(graph_id, candidate.entity_b)
                .await
                .map_err(store_failure)?,
        );
        Ok(CandidateDetail {
            candidate,
            entity_a,
            entity_b,
            decisions,
            identity_a,
            identity_b,
        })
    }

    /// Records a review decision. `same_entity` writes logical identity
    /// metadata only; the graph is never changed.
    ///
    /// # Errors
    ///
    /// [`ResolutionError::InvalidDecision`], [`ApplicationError::GraphNotFound`],
    /// [`ResolutionError::CandidateNotFound`], [`ResolutionError::EntityUnavailable`],
    /// [`ResolutionError::DecisionConflict`], [`ResolutionError::IdentityConflict`],
    /// or a backend failure.
    pub async fn decide(
        &self,
        graph_id: GraphId,
        candidate_id: ResolutionCandidateId,
        request: DecisionRequest,
    ) -> Result<DecisionResult, ApplicationError> {
        match (request.kind, request.canonical_entity_id) {
            (DecisionKind::SameEntity, None) => {
                return Err(ResolutionError::InvalidDecision(
                    "same_entity needs canonical_entity_id: choose which original entity stays canonical"
                        .into(),
                )
                .into());
            }
            (DecisionKind::DifferentEntities, Some(_)) => {
                return Err(ResolutionError::InvalidDecision(
                    "canonical_entity_id is only allowed with same_entity".into(),
                )
                .into());
            }
            _ => {}
        }
        self.explorer.ensure_graph(graph_id).await?;
        let candidate = self
            .store
            .get_candidate(graph_id, candidate_id)
            .await
            .map_err(store_failure)?
            .ok_or(ResolutionError::CandidateNotFound(candidate_id))?;
        if request.kind == DecisionKind::SameEntity {
            // Read-only checks against the graph; nothing is written there.
            let mut entities = vec![candidate.entity_a, candidate.entity_b];
            if let Some(canonical) = request.canonical_entity_id
                && !entities.contains(&canonical)
            {
                entities.push(canonical);
            }
            for entity_id in entities {
                let entity = self
                    .explorer
                    .graph_store()
                    .get_entity(graph_id, entity_id)
                    .await
                    .map_err(graph_failure)?
                    .ok_or(ResolutionError::EntityUnavailable(entity_id))?;
                if entity.entity_type() != &candidate.entity_type {
                    return Err(ResolutionError::IdentityConflict(
                        "entities of different types cannot share an identity".into(),
                    )
                    .into());
                }
            }
        }
        let command = DecisionCommand {
            id: ResolutionDecisionId::new(),
            graph_id,
            candidate_id,
            kind: request.kind,
            canonical: request.canonical_entity_id,
            reconsider: request.reconsider,
            actor: LOCAL_ACTOR.to_owned(),
            decided_at: Utc::now().trunc_subsecs(6),
            max_group_members: self.config.max_group_members,
        };
        let outcome = self
            .store
            .record_decision(command)
            .await
            .map_err(store_failure)?;
        info!(
            graph_id = %graph_id,
            candidate_id = %candidate_id,
            decision = outcome.decision.kind.as_str(),
            replayed = outcome.replayed,
            linked = outcome.decision.linked_entity_ids.len(),
            "entity resolution decision recorded"
        );
        let identity = IdentityView::of(candidate.entity_a, outcome.group.clone());
        Ok(DecisionResult { outcome, identity })
    }

    /// The logical identity of an entity of the graph.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ApplicationError::EntityNotFound`]
    /// (also for an entity of another graph), or a backend failure.
    pub async fn identity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<IdentityView, ApplicationError> {
        self.explorer.get_entity(graph_id, entity_id).await?;
        let stored = self
            .store
            .get_identity_group(graph_id, entity_id)
            .await
            .map_err(store_failure)?;
        Ok(IdentityView::of(entity_id, stored))
    }
}
