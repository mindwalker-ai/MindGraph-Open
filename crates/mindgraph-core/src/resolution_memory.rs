//! The in-memory [`EntityResolutionStore`]: the reference behaviour of the
//! resolution contract, for tests and local development.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mindgraph_domain::{
    EntityId, GraphId, ResolutionCandidateId, ResolutionDecisionId, ResolutionScanId, TypeName,
};
use mindgraph_entity_resolution::IdentityGroup;

use crate::resolution_store::{
    CandidateCounts, CandidatePage, CandidateQuery, CandidateStatus, DecisionCommand, DecisionKind,
    DecisionOutcome, EntityResolutionStore, IdentityMembership, NewCandidate, Repeat,
    ResolutionCandidate, ResolutionDecision, ResolutionScan, ResolutionStoreError, SkippedPairs,
    StoredIdentityGroup, check_new_candidates, check_repeat, decision_record, linked_pair_conflict,
    linked_sides, plan_same_entity, rejected_pair_conflict,
};

/// An in-memory [`EntityResolutionStore`]. One mutex serialises every
/// operation, which mirrors the per-graph serialisation of the PostgreSQL
/// store. Data is lost when the process exits.
#[derive(Debug, Default)]
pub struct MemoryEntityResolutionStore {
    state: Mutex<State>,
}

#[derive(Debug, Clone)]
struct Alias {
    canonical: EntityId,
    entity_type: TypeName,
    decision_id: ResolutionDecisionId,
    linked_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct State {
    scans: BTreeMap<ResolutionScanId, ResolutionScan>,
    candidates: BTreeMap<ResolutionCandidateId, ResolutionCandidate>,
    /// Candidates per `(graph, smaller id, larger id)`.
    pairs: HashMap<(GraphId, EntityId, EntityId), Vec<ResolutionCandidateId>>,
    /// Append-only.
    decisions: Vec<ResolutionDecision>,
    aliases: BTreeMap<(GraphId, EntityId), Alias>,
}

impl State {
    fn pair_has(
        &self,
        graph_id: GraphId,
        a: EntityId,
        b: EntityId,
        status: CandidateStatus,
    ) -> bool {
        self.pairs.get(&(graph_id, a, b)).is_some_and(|ids| {
            ids.iter()
                .any(|id| self.candidates.get(id).is_some_and(|c| c.status == status))
        })
    }

    fn canonical(&self, graph_id: GraphId, entity: EntityId) -> Option<EntityId> {
        self.aliases
            .get(&(graph_id, entity))
            .map(|alias| alias.canonical)
    }

    fn group(&self, graph_id: GraphId, entity: EntityId) -> Option<StoredIdentityGroup> {
        let alias = self.aliases.get(&(graph_id, entity))?;
        let members: Vec<IdentityMembership> = self
            .aliases
            .iter()
            .filter(|((graph, _), row)| *graph == graph_id && row.canonical == alias.canonical)
            .map(|((_, member), row)| IdentityMembership {
                entity_id: *member,
                decision_id: row.decision_id,
                linked_at: row.linked_at,
            })
            .collect();
        Some(StoredIdentityGroup {
            group: IdentityGroup::new(alias.canonical, members.iter().map(|m| m.entity_id)),
            entity_type: alias.entity_type.clone(),
            members,
        })
    }

    fn latest_decision(&self, candidate_id: ResolutionCandidateId) -> Option<&ResolutionDecision> {
        self.decisions
            .iter()
            .rev()
            .find(|decision| decision.candidate_id == candidate_id)
    }

    fn rejected_between(
        &self,
        graph_id: GraphId,
        side_a: &[EntityId],
        side_b: &[EntityId],
        exclude: ResolutionCandidateId,
    ) -> bool {
        self.candidates.values().any(|c| {
            c.graph_id == graph_id
                && c.id != exclude
                && c.status == CandidateStatus::Rejected
                && ((side_a.contains(&c.entity_a) && side_b.contains(&c.entity_b))
                    || (side_a.contains(&c.entity_b) && side_b.contains(&c.entity_a)))
        })
    }
}

impl MemoryEntityResolutionStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // Every method validates before mutating, so state behind a poisoned
    // lock is still consistent.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn ordered(a: &ResolutionCandidate, b: &ResolutionCandidate) -> std::cmp::Ordering {
    b.score.total_cmp(&a.score).then(a.id.cmp(&b.id))
}

#[async_trait]
impl EntityResolutionStore for MemoryEntityResolutionStore {
    async fn create_scan_with_candidates(
        &self,
        mut scan: ResolutionScan,
        candidates: Vec<NewCandidate>,
    ) -> Result<ResolutionScan, ResolutionStoreError> {
        check_new_candidates(&candidates)?;
        let mut state = self.state();
        if state.scans.contains_key(&scan.id)
            || candidates
                .iter()
                .any(|candidate| state.candidates.contains_key(&candidate.id))
        {
            return Err(ResolutionStoreError::InvalidInput(
                "the scan or a candidate id already exists".into(),
            ));
        }
        let graph_id = scan.graph_id;
        let mut skipped = SkippedPairs::default();
        let mut stored = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let (a, b) = (candidate.entity_a, candidate.entity_b);
            if state.pair_has(graph_id, a, b, CandidateStatus::Pending) {
                skipped.already_pending += 1;
            } else if state.pair_has(graph_id, a, b, CandidateStatus::Rejected) {
                skipped.previously_rejected += 1;
            } else if state
                .canonical(graph_id, a)
                .is_some_and(|canonical| state.canonical(graph_id, b) == Some(canonical))
            {
                skipped.already_linked += 1;
            } else {
                stored.push(ResolutionCandidate {
                    id: candidate.id,
                    scan_id: scan.id,
                    graph_id,
                    entity_type: scan.entity_type.clone(),
                    entity_a: a,
                    entity_b: b,
                    score: candidate.score,
                    signals: candidate.signals,
                    status: CandidateStatus::Pending,
                    created_at: scan.created_at,
                    updated_at: scan.created_at,
                    decided_at: None,
                });
            }
        }
        scan.candidates_found = u32::try_from(stored.len()).unwrap_or(u32::MAX);
        scan.skipped = skipped;
        for candidate in stored {
            state
                .pairs
                .entry((graph_id, candidate.entity_a, candidate.entity_b))
                .or_default()
                .push(candidate.id);
            state.candidates.insert(candidate.id, candidate);
        }
        state.scans.insert(scan.id, scan.clone());
        Ok(scan)
    }

    async fn get_scan(
        &self,
        graph_id: GraphId,
        scan_id: ResolutionScanId,
    ) -> Result<Option<ResolutionScan>, ResolutionStoreError> {
        Ok(self
            .state()
            .scans
            .get(&scan_id)
            .filter(|scan| scan.graph_id == graph_id)
            .cloned())
    }

    async fn list_scans(
        &self,
        graph_id: GraphId,
        limit: usize,
    ) -> Result<Vec<ResolutionScan>, ResolutionStoreError> {
        let state = self.state();
        let mut scans: Vec<ResolutionScan> = state
            .scans
            .values()
            .filter(|scan| scan.graph_id == graph_id)
            .cloned()
            .collect();
        scans.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        scans.truncate(limit);
        Ok(scans)
    }

    async fn list_candidates(
        &self,
        graph_id: GraphId,
        query: &CandidateQuery,
    ) -> Result<CandidatePage, ResolutionStoreError> {
        let state = self.state();
        let cursor = match query.after {
            None => None,
            Some(after) => Some(
                state
                    .candidates
                    .get(&after)
                    .filter(|c| c.graph_id == graph_id)
                    .ok_or(ResolutionStoreError::InvalidCursor)?,
            ),
        };
        let scoped: Vec<&ResolutionCandidate> = state
            .candidates
            .values()
            .filter(|c| c.graph_id == graph_id)
            .filter(|c| query.scan_id.is_none_or(|scan| c.scan_id == scan))
            .filter(|c| {
                query
                    .entity_id
                    .is_none_or(|entity| c.entity_a == entity || c.entity_b == entity)
            })
            .collect();
        let mut counts = CandidateCounts::default();
        for candidate in &scoped {
            match candidate.status {
                CandidateStatus::Pending => counts.pending += 1,
                CandidateStatus::Accepted => counts.accepted += 1,
                CandidateStatus::Rejected => counts.rejected += 1,
            }
        }
        let mut rows: Vec<&ResolutionCandidate> = scoped
            .into_iter()
            .filter(|c| query.status.is_none_or(|status| c.status == status))
            .filter(|c| cursor.is_none_or(|cursor| ordered(cursor, c).is_lt()))
            .collect();
        rows.sort_by(|a, b| ordered(a, b));
        let limit = query.limit.max(1);
        let next = (rows.len() > limit).then(|| rows[limit - 1].id);
        rows.truncate(limit);
        Ok(CandidatePage {
            candidates: rows.into_iter().cloned().collect(),
            next,
            counts,
        })
    }

    async fn get_candidate(
        &self,
        graph_id: GraphId,
        candidate_id: ResolutionCandidateId,
    ) -> Result<Option<ResolutionCandidate>, ResolutionStoreError> {
        Ok(self
            .state()
            .candidates
            .get(&candidate_id)
            .filter(|c| c.graph_id == graph_id)
            .cloned())
    }

    async fn list_decisions(
        &self,
        graph_id: GraphId,
        candidate_id: ResolutionCandidateId,
    ) -> Result<Vec<ResolutionDecision>, ResolutionStoreError> {
        Ok(self
            .state()
            .decisions
            .iter()
            .filter(|d| d.graph_id == graph_id && d.candidate_id == candidate_id)
            .cloned()
            .collect())
    }

    async fn record_decision(
        &self,
        command: DecisionCommand,
    ) -> Result<DecisionOutcome, ResolutionStoreError> {
        let mut state = self.state();
        let graph_id = command.graph_id;
        let candidate = state
            .candidates
            .get(&command.candidate_id)
            .filter(|c| c.graph_id == graph_id)
            .cloned()
            .ok_or(ResolutionStoreError::CandidateNotFound(
                command.candidate_id,
            ))?;
        let reconsidered =
            match check_repeat(&candidate, state.latest_decision(candidate.id), &command)? {
                Repeat::Replay(decision) => {
                    let group = state.group(graph_id, candidate.entity_a);
                    return Ok(DecisionOutcome {
                        decision: *decision,
                        candidate,
                        group,
                        replayed: true,
                    });
                }
                Repeat::Proceed { reconsidered } => reconsidered,
            };
        if state.decisions.iter().any(|d| d.id == command.id) {
            return Err(ResolutionStoreError::InvalidInput(
                "the decision id already exists".into(),
            ));
        }
        let stored_a = state.group(graph_id, candidate.entity_a);
        let stored_b = state.group(graph_id, candidate.entity_b);
        if [&stored_a, &stored_b]
            .into_iter()
            .flatten()
            .any(|stored| stored.entity_type != candidate.entity_type)
        {
            return Err(ResolutionStoreError::IdentityConflict(
                "entities of different types cannot share an identity".into(),
            ));
        }
        let group_a = stored_a.as_ref().map(|stored| &stored.group);
        let group_b = stored_b.as_ref().map(|stored| &stored.group);
        let (decision, status) = match command.kind {
            DecisionKind::DifferentEntities => {
                if let (Some(a), Some(b)) = (group_a, group_b)
                    && a.canonical() == b.canonical()
                {
                    return Err(linked_pair_conflict());
                }
                (
                    decision_record(&candidate, &command, None, reconsidered),
                    CandidateStatus::Rejected,
                )
            }
            DecisionKind::SameEntity => {
                let plan = plan_same_entity(&candidate, group_a, group_b, &command)?;
                if let Some((side_a, side_b)) = linked_sides(&candidate, group_a, group_b)
                    && state.rejected_between(graph_id, &side_a, &side_b, candidate.id)
                {
                    return Err(rejected_pair_conflict());
                }
                for member in &plan.changed {
                    state.aliases.insert(
                        (graph_id, *member),
                        Alias {
                            canonical: plan.canonical,
                            entity_type: candidate.entity_type.clone(),
                            decision_id: command.id,
                            linked_at: command.decided_at,
                        },
                    );
                }
                (
                    decision_record(&candidate, &command, Some(&plan), reconsidered),
                    CandidateStatus::Accepted,
                )
            }
        };
        let updated = {
            let stored = state
                .candidates
                .get_mut(&candidate.id)
                .ok_or(ResolutionStoreError::CandidateNotFound(candidate.id))?;
            stored.status = status;
            stored.updated_at = command.decided_at.max(stored.updated_at);
            stored.decided_at = Some(command.decided_at);
            stored.clone()
        };
        state.decisions.push(decision.clone());
        let group = state.group(graph_id, candidate.entity_a);
        Ok(DecisionOutcome {
            decision,
            candidate: updated,
            group,
            replayed: false,
        })
    }

    async fn get_identity_group(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<Option<StoredIdentityGroup>, ResolutionStoreError> {
        Ok(self.state().group(graph_id, entity_id))
    }
}
