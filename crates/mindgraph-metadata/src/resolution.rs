//! `PostgresEntityResolutionStore`: the SQLx implementation of the entity
//! resolution port (M9).
//!
//! Every write takes a transaction-scoped advisory lock for its graph
//! (`pg_advisory_xact_lock`), so scans and decisions of one graph are
//! serialised: two concurrent approvals can never produce conflicting
//! canonical mappings, and the skip rules of a scan see a stable state.
//! Different graphs never wait for each other. The decision rules are the
//! shared functions of `mindgraph_core` (`check_repeat`, `plan_same_entity`,
//! …), applied inside the transaction.
//!
//! The tables hold ids, scores, rule snapshots and value-free evidence;
//! entity properties stay in the GraphStore.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mindgraph_core::matching::{IdentityGroup, MatchSignal, RuleSet, ScanWarning};
use mindgraph_core::{
    CandidateCounts, CandidatePage, CandidateQuery, CandidateStatus, DecisionCommand, DecisionKind,
    DecisionOutcome, EntityResolutionStore, IdentityMembership, MetadataStoreError, NewCandidate,
    Repeat, ResolutionCandidate, ResolutionDecision, ResolutionScan, ResolutionStoreError,
    SkippedPairs, StoredIdentityGroup, check_new_candidates, check_repeat, decision_record,
    linked_pair_conflict, linked_sides, plan_same_entity, rejected_pair_conflict,
};
use mindgraph_domain::{
    EntityId, GraphId, ResolutionCandidateId, ResolutionDecisionId, ResolutionScanId, TypeName,
};
use serde_json::Value;
use sqlx::postgres::{PgPool, PgRow};
// Dynamic SQL here only splices the constant column lists below; every value
// is a bind parameter (`AssertSqlSafe` marks those audited strings).
use sqlx::{AssertSqlSafe, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::store::{corrupt, map_error, version_column, version_from_column};

/// The SQLx [`EntityResolutionStore`], sharing the metadata database pool.
#[derive(Debug, Clone)]
pub struct PostgresEntityResolutionStore {
    pool: PgPool,
}

impl PostgresEntityResolutionStore {
    /// Wraps an existing pool whose database has the M9 migration applied
    /// (see `PostgresMetadataStore::migrate`).
    #[must_use]
    pub const fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

type Tx<'a> = Transaction<'a, Postgres>;

fn sql(context: &'static str) -> impl Fn(sqlx::Error) -> ResolutionStoreError {
    move |err| ResolutionStoreError::Store(map_error(context, err))
}

fn encode<T: serde::Serialize>(what: &str, value: &T) -> Result<Value, ResolutionStoreError> {
    serde_json::to_value(value).map_err(|err| {
        ResolutionStoreError::Store(MetadataStoreError::Serialization(format!("{what}: {err}")))
    })
}

fn decode<T: serde::de::DeserializeOwned>(
    what: &str,
    value: Value,
) -> Result<T, ResolutionStoreError> {
    serde_json::from_value(value).map_err(|err| ResolutionStoreError::Store(corrupt(what, err)))
}

fn id<T: TryFrom<Uuid>>(what: &str, uuid: Uuid) -> Result<T, ResolutionStoreError>
where
    T::Error: std::fmt::Display,
{
    T::try_from(uuid).map_err(|err| ResolutionStoreError::Store(corrupt(what, err)))
}

fn ids<T: TryFrom<Uuid>>(what: &str, uuids: Vec<Uuid>) -> Result<Vec<T>, ResolutionStoreError>
where
    T::Error: std::fmt::Display,
{
    uuids.into_iter().map(|uuid| id(what, uuid)).collect()
}

fn int(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn unsigned(what: &str, value: i32) -> Result<u32, ResolutionStoreError> {
    u32::try_from(value).map_err(|err| ResolutionStoreError::Store(corrupt(what, err)))
}

fn type_name(what: &str, value: String) -> Result<TypeName, ResolutionStoreError> {
    TypeName::new(value).map_err(|err| ResolutionStoreError::Store(corrupt(what, err)))
}

/// Serialises writes of one graph until the transaction ends.
async fn lock_graph(tx: &mut Tx<'_>, graph_id: GraphId) -> Result<(), ResolutionStoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("mindgraph.entity_resolution:{graph_id}"))
        .execute(&mut **tx)
        .await
        .map_err(sql("locking the graph for entity resolution"))?;
    Ok(())
}

const SCAN_COLUMNS: &str = "id, graph_id, entity_type, rules_json, ontology_version, max_entities, \
     entities_examined, truncated, pairs_compared, comparison_budget, comparison_budget_reached, \
     oversized_blocks, candidate_limit_reached, candidates_found, skipped_pending, skipped_rejected, \
     skipped_linked, warnings_json, duration_ms, created_at";

const CANDIDATE_COLUMNS: &str = "id, scan_id, graph_id, entity_type, entity_low_id, entity_high_id, \
     score, signals_json, status, created_at, updated_at, decided_at";

const DECISION_COLUMNS: &str = "id, graph_id, candidate_id, scan_id, decision, entity_low_id, \
     entity_high_id, canonical_entity_id, previous_canonical_ids, linked_entity_ids, score, \
     matched_rules, actor, reconsidered, created_at";

fn decode_scan(row: &PgRow) -> Result<ResolutionScan, ResolutionStoreError> {
    let read = sql("reading a resolution scan");
    let get_u32 = |column: &str| -> Result<u32, ResolutionStoreError> {
        unsigned(column, row.try_get::<i32, _>(column).map_err(&read)?)
    };
    let duration: i64 = row.try_get("duration_ms").map_err(&read)?;
    Ok(ResolutionScan {
        id: id("scan id", row.try_get("id").map_err(&read)?)?,
        graph_id: id("graph id", row.try_get("graph_id").map_err(&read)?)?,
        entity_type: type_name(
            "scan entity type",
            row.try_get("entity_type").map_err(&read)?,
        )?,
        rules: decode::<RuleSet>("scan rules", row.try_get("rules_json").map_err(&read)?)?,
        ontology_version: version_from_column(row.try_get("ontology_version").map_err(&read)?)?,
        max_entities: get_u32("max_entities")?,
        entities_examined: get_u32("entities_examined")?,
        truncated: row.try_get("truncated").map_err(&read)?,
        pairs_compared: get_u32("pairs_compared")?,
        comparison_budget: get_u32("comparison_budget")?,
        comparison_budget_reached: row.try_get("comparison_budget_reached").map_err(&read)?,
        oversized_blocks: get_u32("oversized_blocks")?,
        candidate_limit_reached: row.try_get("candidate_limit_reached").map_err(&read)?,
        candidates_found: get_u32("candidates_found")?,
        skipped: SkippedPairs {
            already_pending: get_u32("skipped_pending")?,
            previously_rejected: get_u32("skipped_rejected")?,
            already_linked: get_u32("skipped_linked")?,
        },
        warnings: decode::<Vec<ScanWarning>>(
            "scan warnings",
            row.try_get("warnings_json").map_err(&read)?,
        )?,
        duration_ms: u64::try_from(duration)
            .map_err(|err| ResolutionStoreError::Store(corrupt("scan duration", err)))?,
        created_at: row.try_get("created_at").map_err(&read)?,
    })
}

fn decode_candidate(row: &PgRow) -> Result<ResolutionCandidate, ResolutionStoreError> {
    let read = sql("reading a resolution candidate");
    let status: String = row.try_get("status").map_err(&read)?;
    Ok(ResolutionCandidate {
        id: id("candidate id", row.try_get("id").map_err(&read)?)?,
        scan_id: id("scan id", row.try_get("scan_id").map_err(&read)?)?,
        graph_id: id("graph id", row.try_get("graph_id").map_err(&read)?)?,
        entity_type: type_name(
            "candidate entity type",
            row.try_get("entity_type").map_err(&read)?,
        )?,
        entity_a: id("entity id", row.try_get("entity_low_id").map_err(&read)?)?,
        entity_b: id("entity id", row.try_get("entity_high_id").map_err(&read)?)?,
        score: row.try_get("score").map_err(&read)?,
        signals: decode::<Vec<MatchSignal>>(
            "candidate signals",
            row.try_get("signals_json").map_err(&read)?,
        )?,
        status: CandidateStatus::parse(&status)
            .ok_or_else(|| ResolutionStoreError::Store(corrupt("candidate status", &status)))?,
        created_at: row.try_get("created_at").map_err(&read)?,
        updated_at: row.try_get("updated_at").map_err(&read)?,
        decided_at: row.try_get("decided_at").map_err(&read)?,
    })
}

fn decode_decision(row: &PgRow) -> Result<ResolutionDecision, ResolutionStoreError> {
    let read = sql("reading a resolution decision");
    let kind: String = row.try_get("decision").map_err(&read)?;
    let canonical: Option<Uuid> = row.try_get("canonical_entity_id").map_err(&read)?;
    Ok(ResolutionDecision {
        id: id("decision id", row.try_get("id").map_err(&read)?)?,
        graph_id: id("graph id", row.try_get("graph_id").map_err(&read)?)?,
        candidate_id: id("candidate id", row.try_get("candidate_id").map_err(&read)?)?,
        scan_id: id("scan id", row.try_get("scan_id").map_err(&read)?)?,
        kind: DecisionKind::parse(&kind)
            .ok_or_else(|| ResolutionStoreError::Store(corrupt("decision kind", &kind)))?,
        entity_a: id("entity id", row.try_get("entity_low_id").map_err(&read)?)?,
        entity_b: id("entity id", row.try_get("entity_high_id").map_err(&read)?)?,
        canonical_entity_id: canonical.map(|uuid| id("entity id", uuid)).transpose()?,
        previous_canonical_ids: ids(
            "entity id",
            row.try_get("previous_canonical_ids").map_err(&read)?,
        )?,
        linked_entity_ids: ids(
            "entity id",
            row.try_get("linked_entity_ids").map_err(&read)?,
        )?,
        score: row.try_get("score").map_err(&read)?,
        matched_rules: row.try_get("matched_rules").map_err(&read)?,
        actor: row.try_get("actor").map_err(&read)?,
        reconsidered: row.try_get("reconsidered").map_err(&read)?,
        created_at: row.try_get("created_at").map_err(&read)?,
    })
}

fn uuids(entities: &[EntityId]) -> Vec<Uuid> {
    entities.iter().map(EntityId::as_uuid).collect()
}

/// The identity group of `entity_id`, read through `executor`.
async fn load_group<'e, E>(
    executor: E,
    graph_id: GraphId,
    entity_id: EntityId,
) -> Result<Option<StoredIdentityGroup>, ResolutionStoreError>
where
    E: sqlx::PgExecutor<'e>,
{
    let rows = sqlx::query(
        "SELECT m.entity_id, m.canonical_entity_id, m.entity_type, m.decision_id, m.updated_at \
         FROM entity_identity_aliases a \
         JOIN entity_identity_aliases m \
           ON m.graph_id = a.graph_id AND m.canonical_entity_id = a.canonical_entity_id \
         WHERE a.graph_id = $1 AND a.entity_id = $2 \
         ORDER BY m.entity_id",
    )
    .bind(graph_id.as_uuid())
    .bind(entity_id.as_uuid())
    .fetch_all(executor)
    .await
    .map_err(sql("reading an identity group"))?;
    let read = sql("reading an identity group");
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let canonical: EntityId = id(
        "entity id",
        first.try_get("canonical_entity_id").map_err(&read)?,
    )?;
    let entity_type = type_name(
        "group entity type",
        first.try_get("entity_type").map_err(&read)?,
    )?;
    let mut members = Vec::with_capacity(rows.len());
    for row in &rows {
        let linked_at: DateTime<Utc> = row.try_get("updated_at").map_err(&read)?;
        members.push(IdentityMembership {
            entity_id: id("entity id", row.try_get("entity_id").map_err(&read)?)?,
            decision_id: id::<ResolutionDecisionId>(
                "decision id",
                row.try_get("decision_id").map_err(&read)?,
            )?,
            linked_at,
        });
    }
    Ok(Some(StoredIdentityGroup {
        group: IdentityGroup::new(canonical, members.iter().map(|m| m.entity_id)),
        entity_type,
        members,
    }))
}

#[async_trait]
impl EntityResolutionStore for PostgresEntityResolutionStore {
    async fn create_scan_with_candidates(
        &self,
        mut scan: ResolutionScan,
        candidates: Vec<NewCandidate>,
    ) -> Result<ResolutionScan, ResolutionStoreError> {
        check_new_candidates(&candidates)?;
        let graph_id = scan.graph_id;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(sql("starting a transaction"))?;
        lock_graph(&mut tx, graph_id).await?;

        let lows: Vec<Uuid> = candidates.iter().map(|c| c.entity_a.as_uuid()).collect();
        let highs: Vec<Uuid> = candidates.iter().map(|c| c.entity_b.as_uuid()).collect();
        let existing = sqlx::query(
            "SELECT c.entity_low_id, c.entity_high_id, c.status \
             FROM entity_resolution_candidates c \
             JOIN UNNEST($2::uuid[], $3::uuid[]) AS p(low, high) \
               ON c.entity_low_id = p.low AND c.entity_high_id = p.high \
             WHERE c.graph_id = $1 AND c.status IN ('pending', 'rejected')",
        )
        .bind(graph_id.as_uuid())
        .bind(&lows)
        .bind(&highs)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql("reading earlier candidates"))?;
        let read = sql("reading earlier candidates");
        let mut pending = HashSet::new();
        let mut rejected = HashSet::new();
        for row in &existing {
            let pair: (Uuid, Uuid) = (
                row.try_get("entity_low_id").map_err(&read)?,
                row.try_get("entity_high_id").map_err(&read)?,
            );
            let status: String = row.try_get("status").map_err(&read)?;
            if status == "pending" {
                pending.insert(pair);
            } else {
                rejected.insert(pair);
            }
        }
        let involved: Vec<Uuid> = lows
            .iter()
            .chain(&highs)
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let aliases = sqlx::query(
            "SELECT entity_id, canonical_entity_id FROM entity_identity_aliases \
             WHERE graph_id = $1 AND entity_id = ANY($2)",
        )
        .bind(graph_id.as_uuid())
        .bind(&involved)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql("reading identity groups"))?;
        let mut canonical: HashMap<Uuid, Uuid> = HashMap::new();
        for row in &aliases {
            canonical.insert(
                row.try_get("entity_id").map_err(&read)?,
                row.try_get("canonical_entity_id").map_err(&read)?,
            );
        }

        let mut skipped = SkippedPairs::default();
        let mut stored = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let pair = (candidate.entity_a.as_uuid(), candidate.entity_b.as_uuid());
            if pending.contains(&pair) {
                skipped.already_pending += 1;
            } else if rejected.contains(&pair) {
                skipped.previously_rejected += 1;
            } else if canonical
                .get(&pair.0)
                .is_some_and(|c| canonical.get(&pair.1) == Some(c))
            {
                skipped.already_linked += 1;
            } else {
                stored.push(candidate);
            }
        }
        scan.candidates_found = u32::try_from(stored.len()).unwrap_or(u32::MAX);
        scan.skipped = skipped;

        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO entity_resolution_scans ({SCAN_COLUMNS}) VALUES \
             ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)"
        )))
        .bind(scan.id.as_uuid())
        .bind(graph_id.as_uuid())
        .bind(scan.entity_type.as_str())
        .bind(encode("scan rules", &scan.rules)?)
        .bind(version_column(scan.ontology_version)?)
        .bind(int(scan.max_entities))
        .bind(int(scan.entities_examined))
        .bind(scan.truncated)
        .bind(int(scan.pairs_compared))
        .bind(int(scan.comparison_budget))
        .bind(scan.comparison_budget_reached)
        .bind(int(scan.oversized_blocks))
        .bind(scan.candidate_limit_reached)
        .bind(int(scan.candidates_found))
        .bind(int(scan.skipped.already_pending))
        .bind(int(scan.skipped.previously_rejected))
        .bind(int(scan.skipped.already_linked))
        .bind(encode("scan warnings", &scan.warnings)?)
        .bind(i64::try_from(scan.duration_ms).unwrap_or(i64::MAX))
        .bind(scan.created_at)
        .execute(&mut *tx)
        .await
        .map_err(sql("storing the resolution scan"))?;

        if !stored.is_empty() {
            let ids: Vec<Uuid> = stored.iter().map(|c| c.id.as_uuid()).collect();
            let lows: Vec<Uuid> = stored.iter().map(|c| c.entity_a.as_uuid()).collect();
            let highs: Vec<Uuid> = stored.iter().map(|c| c.entity_b.as_uuid()).collect();
            let scores: Vec<f64> = stored.iter().map(|c| c.score).collect();
            let signals = stored
                .iter()
                .map(|c| encode("candidate signals", &c.signals))
                .collect::<Result<Vec<Value>, _>>()?;
            sqlx::query(
                "INSERT INTO entity_resolution_candidates \
                 (id, scan_id, graph_id, entity_type, entity_low_id, entity_high_id, score, \
                  signals_json, status, created_at, updated_at) \
                 SELECT t.id, $1, $2, $3, t.low, t.high, t.score, t.signals, 'pending', $4, $4 \
                 FROM UNNEST($5::uuid[], $6::uuid[], $7::uuid[], $8::double precision[], $9::jsonb[]) \
                   AS t(id, low, high, score, signals)",
            )
            .bind(scan.id.as_uuid())
            .bind(graph_id.as_uuid())
            .bind(scan.entity_type.as_str())
            .bind(scan.created_at)
            .bind(&ids)
            .bind(&lows)
            .bind(&highs)
            .bind(&scores)
            .bind(&signals)
            .execute(&mut *tx)
            .await
            .map_err(sql("storing resolution candidates"))?;
        }
        tx.commit()
            .await
            .map_err(sql("committing the resolution scan"))?;
        Ok(scan)
    }

    async fn get_scan(
        &self,
        graph_id: GraphId,
        scan_id: ResolutionScanId,
    ) -> Result<Option<ResolutionScan>, ResolutionStoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "SELECT {SCAN_COLUMNS} FROM entity_resolution_scans WHERE id = $1 AND graph_id = $2"
        )))
        .bind(scan_id.as_uuid())
        .bind(graph_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(sql("reading a resolution scan"))?
        .as_ref()
        .map(decode_scan)
        .transpose()
    }

    async fn list_scans(
        &self,
        graph_id: GraphId,
        limit: usize,
    ) -> Result<Vec<ResolutionScan>, ResolutionStoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "SELECT {SCAN_COLUMNS} FROM entity_resolution_scans WHERE graph_id = $1 \
             ORDER BY created_at DESC, id DESC LIMIT $2"
        )))
        .bind(graph_id.as_uuid())
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .map_err(sql("listing resolution scans"))?
        .iter()
        .map(decode_scan)
        .collect()
    }

    async fn list_candidates(
        &self,
        graph_id: GraphId,
        query: &CandidateQuery,
    ) -> Result<CandidatePage, ResolutionStoreError> {
        let cursor: Option<(f64, Uuid)> = match query.after {
            None => None,
            Some(after) => {
                let row = sqlx::query(
                    "SELECT score FROM entity_resolution_candidates WHERE id = $1 AND graph_id = $2",
                )
                .bind(after.as_uuid())
                .bind(graph_id.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(sql("reading the candidate cursor"))?
                .ok_or(ResolutionStoreError::InvalidCursor)?;
                let score: f64 = row
                    .try_get("score")
                    .map_err(sql("reading the candidate cursor"))?;
                Some((score, after.as_uuid()))
            }
        };
        let scan = query.scan_id.map(|id| id.as_uuid());
        let entity = query.entity_id.map(|id| id.as_uuid());
        let limit = query.limit.max(1);
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT {CANDIDATE_COLUMNS} FROM entity_resolution_candidates \
             WHERE graph_id = $1 \
               AND ($2::text IS NULL OR status = $2) \
               AND ($3::uuid IS NULL OR scan_id = $3) \
               AND ($4::uuid IS NULL OR entity_low_id = $4 OR entity_high_id = $4) \
               AND ($5::double precision IS NULL OR score < $5 OR (score = $5 AND id > $6)) \
             ORDER BY score DESC, id ASC LIMIT $7"
        )))
        .bind(graph_id.as_uuid())
        .bind(query.status.map(CandidateStatus::as_str))
        .bind(scan)
        .bind(entity)
        .bind(cursor.map(|(score, _)| score))
        .bind(cursor.map(|(_, id)| id))
        .bind(i64::try_from(limit + 1).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .map_err(sql("listing resolution candidates"))?;
        let mut candidates = rows
            .iter()
            .map(decode_candidate)
            .collect::<Result<Vec<_>, _>>()?;
        let next = (candidates.len() > limit).then(|| candidates[limit - 1].id);
        candidates.truncate(limit);

        let count_rows = sqlx::query(
            "SELECT status, count(*) AS n FROM entity_resolution_candidates \
             WHERE graph_id = $1 \
               AND ($2::uuid IS NULL OR scan_id = $2) \
               AND ($3::uuid IS NULL OR entity_low_id = $3 OR entity_high_id = $3) \
             GROUP BY status",
        )
        .bind(graph_id.as_uuid())
        .bind(scan)
        .bind(entity)
        .fetch_all(&self.pool)
        .await
        .map_err(sql("counting resolution candidates"))?;
        let read = sql("counting resolution candidates");
        let mut counts = CandidateCounts::default();
        for row in &count_rows {
            let status: String = row.try_get("status").map_err(&read)?;
            let n = u64::try_from(row.try_get::<i64, _>("n").map_err(&read)?).unwrap_or(0);
            match CandidateStatus::parse(&status) {
                Some(CandidateStatus::Pending) => counts.pending = n,
                Some(CandidateStatus::Accepted) => counts.accepted = n,
                Some(CandidateStatus::Rejected) => counts.rejected = n,
                None => {
                    return Err(ResolutionStoreError::Store(corrupt(
                        "candidate status",
                        &status,
                    )));
                }
            }
        }
        Ok(CandidatePage {
            candidates,
            next,
            counts,
        })
    }

    async fn get_candidate(
        &self,
        graph_id: GraphId,
        candidate_id: ResolutionCandidateId,
    ) -> Result<Option<ResolutionCandidate>, ResolutionStoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "SELECT {CANDIDATE_COLUMNS} FROM entity_resolution_candidates \
             WHERE id = $1 AND graph_id = $2"
        )))
        .bind(candidate_id.as_uuid())
        .bind(graph_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(sql("reading a resolution candidate"))?
        .as_ref()
        .map(decode_candidate)
        .transpose()
    }

    async fn list_decisions(
        &self,
        graph_id: GraphId,
        candidate_id: ResolutionCandidateId,
    ) -> Result<Vec<ResolutionDecision>, ResolutionStoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "SELECT {DECISION_COLUMNS} FROM entity_resolution_decisions \
             WHERE graph_id = $1 AND candidate_id = $2 ORDER BY created_at, id"
        )))
        .bind(graph_id.as_uuid())
        .bind(candidate_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(sql("listing resolution decisions"))?
        .iter()
        .map(decode_decision)
        .collect()
    }

    #[allow(clippy::too_many_lines)] // one transaction, read top to bottom
    async fn record_decision(
        &self,
        command: DecisionCommand,
    ) -> Result<DecisionOutcome, ResolutionStoreError> {
        let graph_id = command.graph_id;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(sql("starting a transaction"))?;
        lock_graph(&mut tx, graph_id).await?;

        let candidate = sqlx::query(AssertSqlSafe(format!(
            "SELECT {CANDIDATE_COLUMNS} FROM entity_resolution_candidates \
             WHERE id = $1 AND graph_id = $2 FOR UPDATE"
        )))
        .bind(command.candidate_id.as_uuid())
        .bind(graph_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(sql("reading a resolution candidate"))?
        .as_ref()
        .map(decode_candidate)
        .transpose()?
        .ok_or(ResolutionStoreError::CandidateNotFound(
            command.candidate_id,
        ))?;
        let latest = sqlx::query(AssertSqlSafe(format!(
            "SELECT {DECISION_COLUMNS} FROM entity_resolution_decisions \
             WHERE candidate_id = $1 AND graph_id = $2 ORDER BY created_at DESC, id DESC LIMIT 1"
        )))
        .bind(candidate.id.as_uuid())
        .bind(graph_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(sql("reading the latest decision"))?
        .as_ref()
        .map(decode_decision)
        .transpose()?;

        let reconsidered = match check_repeat(&candidate, latest.as_ref(), &command)? {
            Repeat::Replay(decision) => {
                let group = load_group(&mut *tx, graph_id, candidate.entity_a).await?;
                tx.commit().await.map_err(sql("ending the transaction"))?;
                return Ok(DecisionOutcome {
                    decision: *decision,
                    candidate,
                    group,
                    replayed: true,
                });
            }
            Repeat::Proceed { reconsidered } => reconsidered,
        };

        let stored_a = load_group(&mut *tx, graph_id, candidate.entity_a).await?;
        let stored_b = load_group(&mut *tx, graph_id, candidate.entity_b).await?;
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

        let (decision, plan, status) = match command.kind {
            DecisionKind::DifferentEntities => {
                if let (Some(a), Some(b)) = (group_a, group_b)
                    && a.canonical() == b.canonical()
                {
                    return Err(linked_pair_conflict());
                }
                (
                    decision_record(&candidate, &command, None, reconsidered),
                    None,
                    CandidateStatus::Rejected,
                )
            }
            DecisionKind::SameEntity => {
                let plan = plan_same_entity(&candidate, group_a, group_b, &command)?;
                if let Some((side_a, side_b)) = linked_sides(&candidate, group_a, group_b) {
                    let conflict = sqlx::query(
                        "SELECT 1 FROM entity_resolution_candidates \
                         WHERE graph_id = $1 AND status = 'rejected' AND id <> $2 \
                           AND ((entity_low_id = ANY($3) AND entity_high_id = ANY($4)) \
                             OR (entity_low_id = ANY($4) AND entity_high_id = ANY($3))) \
                         LIMIT 1",
                    )
                    .bind(graph_id.as_uuid())
                    .bind(candidate.id.as_uuid())
                    .bind(uuids(&side_a))
                    .bind(uuids(&side_b))
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(sql("checking earlier rejections"))?;
                    if conflict.is_some() {
                        return Err(rejected_pair_conflict());
                    }
                }
                (
                    decision_record(&candidate, &command, Some(&plan), reconsidered),
                    Some(plan),
                    CandidateStatus::Accepted,
                )
            }
        };

        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO entity_resolution_decisions ({DECISION_COLUMNS}) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)"
        )))
        .bind(decision.id.as_uuid())
        .bind(graph_id.as_uuid())
        .bind(decision.candidate_id.as_uuid())
        .bind(decision.scan_id.as_uuid())
        .bind(decision.kind.as_str())
        .bind(decision.entity_a.as_uuid())
        .bind(decision.entity_b.as_uuid())
        .bind(decision.canonical_entity_id.map(|id| id.as_uuid()))
        .bind(uuids(&decision.previous_canonical_ids))
        .bind(uuids(&decision.linked_entity_ids))
        .bind(decision.score)
        .bind(&decision.matched_rules)
        .bind(&decision.actor)
        .bind(decision.reconsidered)
        .bind(decision.created_at)
        .execute(&mut *tx)
        .await
        .map_err(sql("storing the resolution decision"))?;

        if let Some(plan) = plan.as_ref().filter(|plan| !plan.is_noop()) {
            // Every changed member maps onto the canonical entity. The
            // flatness trigger checks the whole group at commit.
            sqlx::query(
                "INSERT INTO entity_identity_aliases \
                 (graph_id, entity_id, canonical_entity_id, entity_type, decision_id, created_at, updated_at) \
                 SELECT $1, member, $2, $3, $4, $5, $5 FROM UNNEST($6::uuid[]) AS member \
                 ON CONFLICT (graph_id, entity_id) DO UPDATE SET \
                   canonical_entity_id = EXCLUDED.canonical_entity_id, \
                   entity_type = EXCLUDED.entity_type, \
                   decision_id = EXCLUDED.decision_id, \
                   updated_at = GREATEST(entity_identity_aliases.created_at, EXCLUDED.updated_at)",
            )
            .bind(graph_id.as_uuid())
            .bind(plan.canonical.as_uuid())
            .bind(candidate.entity_type.as_str())
            .bind(decision.id.as_uuid())
            .bind(decision.created_at)
            .bind(uuids(&plan.changed))
            .execute(&mut *tx)
            .await
            .map_err(sql("storing identity mappings"))?;
        }

        let updated = sqlx::query(AssertSqlSafe(format!(
            "UPDATE entity_resolution_candidates \
             SET status = $3, updated_at = GREATEST(updated_at, $4), decided_at = $4 \
             WHERE id = $1 AND graph_id = $2 RETURNING {CANDIDATE_COLUMNS}"
        )))
        .bind(candidate.id.as_uuid())
        .bind(graph_id.as_uuid())
        .bind(status.as_str())
        .bind(decision.created_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(sql("updating the resolution candidate"))?;
        let updated = decode_candidate(&updated)?;
        let group = load_group(&mut *tx, graph_id, candidate.entity_a).await?;
        // The deferred flatness checks run here; a violation rolls back the
        // decision, the mappings and the candidate update together.
        tx.commit()
            .await
            .map_err(sql("committing the resolution decision"))?;
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
        load_group(&self.pool, graph_id, entity_id).await
    }
}
