//! Entity resolution endpoints (M9): bounded duplicate scans, candidate
//! review and logical identity groups.
//!
//! ```text
//! POST /api/v1/graphs/{graph_id}/resolution/scans                  run a bounded scan (201)
//! GET  /api/v1/graphs/{graph_id}/resolution/scans?limit=10         recent scans
//! GET  /api/v1/graphs/{graph_id}/resolution/scans/{scan_id}        one scan
//! GET  /api/v1/graphs/{graph_id}/resolution/candidates             review queue (paginated)
//! GET  /api/v1/graphs/{graph_id}/resolution/candidates/{id}        both entities + evidence
//! POST /api/v1/graphs/{graph_id}/resolution/candidates/{id}/decisions   same / different
//! GET  /api/v1/graphs/{graph_id}/resolution/entities/{entity_id}   logical identity group
//! ```
//!
//! A `same_entity` decision is a logical identity mapping stored as
//! metadata. Every response that describes identity says
//! `"physical_graph_merged": false`: the original entities and relationships
//! stay in the graph, and no other endpoint substitutes canonical ids.

use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use mindgraph_core::matching::{MatchRule, MatchSignal, RuleSpec, ScanWarning};
use mindgraph_core::{
    CandidateDetail, CandidateQuery, CandidateStatus, DEFAULT_CANDIDATE_PAGE, DecisionKind,
    DecisionRequest, EntityResolutionService, IdentityView, ResolutionCandidate,
    ResolutionDecision, ResolutionScan, ScanRequest,
};
use mindgraph_domain::{EntityId, GraphId, ResolutionCandidateId, ResolutionScanId};
use serde::{Deserialize, Serialize};

use crate::dto::timestamp;
use crate::error::ApiError;
use crate::explorer::EntityDto;
use crate::routes::AppState;

/// Largest scan request body: a handful of rules, never data.
pub(crate) const MAX_SCAN_BODY_BYTES: usize = 16 * 1024;

// --- requests --------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleBody {
    property: String,
    kind: String,
    #[serde(default)]
    threshold: Option<f64>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    case_insensitive_local_part: Option<bool>,
}

/// `POST .../resolution/scans`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScanBody {
    entity_type: String,
    #[serde(default)]
    max_entities: Option<usize>,
    rules: Vec<RuleBody>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScanListParams {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CandidateParams {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    scan_id: Option<String>,
    #[serde(default)]
    entity_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

/// `POST .../candidates/{id}/decisions`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecisionBody {
    decision: String,
    #[serde(default)]
    canonical_entity_id: Option<String>,
    #[serde(default)]
    reconsider: bool,
}

// --- responses -------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct SkippedDto {
    already_pending: u32,
    previously_rejected: u32,
    already_linked: u32,
}

/// A scan and its coverage. `exhaustive` is `true` only when every entity
/// of the type was examined and no limit cut the comparison short.
#[derive(Debug, Serialize)]
pub(crate) struct ScanDto {
    scan_id: String,
    graph_id: String,
    entity_type: String,
    ontology_version: u32,
    rules: Vec<MatchRule>,
    max_entities: u32,
    entities_examined: u32,
    truncated: bool,
    pairs_compared: u32,
    comparison_budget: u32,
    comparison_budget_reached: bool,
    oversized_blocks: u32,
    candidate_limit_reached: bool,
    candidates_found: u32,
    skipped: SkippedDto,
    exhaustive: bool,
    warnings: Vec<ScanWarning>,
    duration_ms: u64,
    created_at: String,
}

impl From<&ResolutionScan> for ScanDto {
    fn from(scan: &ResolutionScan) -> Self {
        Self {
            scan_id: scan.id.to_string(),
            graph_id: scan.graph_id.to_string(),
            entity_type: scan.entity_type.to_string(),
            ontology_version: scan.ontology_version.get(),
            rules: scan.rules.rules.clone(),
            max_entities: scan.max_entities,
            entities_examined: scan.entities_examined,
            truncated: scan.truncated,
            pairs_compared: scan.pairs_compared,
            comparison_budget: scan.comparison_budget,
            comparison_budget_reached: scan.comparison_budget_reached,
            oversized_blocks: scan.oversized_blocks,
            candidate_limit_reached: scan.candidate_limit_reached,
            candidates_found: scan.candidates_found,
            skipped: SkippedDto {
                already_pending: scan.skipped.already_pending,
                previously_rejected: scan.skipped.previously_rejected,
                already_linked: scan.skipped.already_linked,
            },
            exhaustive: scan.exhaustive(),
            warnings: scan.warnings.clone(),
            duration_ms: scan.duration_ms,
            created_at: timestamp(scan.created_at),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ScanListDto {
    data: Vec<ScanDto>,
}

/// A potential duplicate. `score` is a ranking heuristic, not a probability.
#[derive(Debug, Serialize)]
pub(crate) struct CandidateDto {
    id: String,
    scan_id: String,
    graph_id: String,
    entity_type: String,
    entity_a_id: String,
    entity_b_id: String,
    score: f64,
    signals: Vec<MatchSignal>,
    matched_rules: Vec<String>,
    status: &'static str,
    created_at: String,
    updated_at: String,
    decided_at: Option<String>,
}

impl From<&ResolutionCandidate> for CandidateDto {
    fn from(candidate: &ResolutionCandidate) -> Self {
        Self {
            id: candidate.id.to_string(),
            scan_id: candidate.scan_id.to_string(),
            graph_id: candidate.graph_id.to_string(),
            entity_type: candidate.entity_type.to_string(),
            entity_a_id: candidate.entity_a.to_string(),
            entity_b_id: candidate.entity_b.to_string(),
            score: candidate.score,
            signals: candidate.signals.clone(),
            matched_rules: candidate.matched_rules(),
            status: candidate.status.as_str(),
            created_at: timestamp(candidate.created_at),
            updated_at: timestamp(candidate.updated_at),
            decided_at: candidate.decided_at.map(timestamp),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct CountsDto {
    pending: u64,
    accepted: u64,
    rejected: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct CandidateListDto {
    data: Vec<CandidateDto>,
    next_cursor: Option<String>,
    limit: usize,
    counts: CountsDto,
}

/// A review decision with its provenance.
#[derive(Debug, Serialize)]
pub(crate) struct DecisionDto {
    id: String,
    candidate_id: String,
    scan_id: String,
    decision: &'static str,
    entity_a_id: String,
    entity_b_id: String,
    canonical_entity_id: Option<String>,
    previous_canonical_entity_ids: Vec<String>,
    linked_entity_ids: Vec<String>,
    score: f64,
    matched_rules: Vec<String>,
    actor: String,
    reconsidered: bool,
    created_at: String,
}

fn strings(ids: &[EntityId]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

impl From<&ResolutionDecision> for DecisionDto {
    fn from(decision: &ResolutionDecision) -> Self {
        Self {
            id: decision.id.to_string(),
            candidate_id: decision.candidate_id.to_string(),
            scan_id: decision.scan_id.to_string(),
            decision: decision.kind.as_str(),
            entity_a_id: decision.entity_a.to_string(),
            entity_b_id: decision.entity_b.to_string(),
            canonical_entity_id: decision.canonical_entity_id.map(|id| id.to_string()),
            previous_canonical_entity_ids: strings(&decision.previous_canonical_ids),
            linked_entity_ids: strings(&decision.linked_entity_ids),
            score: decision.score,
            matched_rules: decision.matched_rules.clone(),
            actor: decision.actor.clone(),
            reconsidered: decision.reconsidered,
            created_at: timestamp(decision.created_at),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct MemberDto {
    entity_id: String,
    decision_id: String,
    linked_at: String,
}

/// The logical identity of one entity. Always `physical_graph_merged:
/// false`: the graph keeps every original entity and relationship.
#[derive(Debug, Serialize)]
pub(crate) struct IdentityDto {
    entity_id: String,
    canonical_entity_id: String,
    member_entity_ids: Vec<String>,
    is_canonical: bool,
    resolved: bool,
    resolution_type: Option<&'static str>,
    entity_type: Option<String>,
    members: Vec<MemberDto>,
    physical_graph_merged: bool,
}

impl From<&IdentityView> for IdentityDto {
    fn from(view: &IdentityView) -> Self {
        Self {
            entity_id: view.entity_id.to_string(),
            canonical_entity_id: view.canonical_entity_id.to_string(),
            member_entity_ids: strings(&view.member_ids()),
            is_canonical: view.is_canonical(),
            resolved: view.resolved(),
            resolution_type: view.resolved().then_some("manual"),
            entity_type: view.entity_type.as_ref().map(ToString::to_string),
            members: view
                .members
                .iter()
                .map(|member| MemberDto {
                    entity_id: member.entity_id.to_string(),
                    decision_id: member.decision_id.to_string(),
                    linked_at: timestamp(member.linked_at),
                })
                .collect(),
            physical_graph_merged: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct CandidateDetailDto {
    candidate: CandidateDto,
    entity_a: Option<EntityDto>,
    entity_b: Option<EntityDto>,
    identity_a: IdentityDto,
    identity_b: IdentityDto,
    decisions: Vec<DecisionDto>,
}

impl From<&CandidateDetail> for CandidateDetailDto {
    fn from(detail: &CandidateDetail) -> Self {
        Self {
            candidate: CandidateDto::from(&detail.candidate),
            entity_a: detail.entity_a.as_ref().map(EntityDto::from),
            entity_b: detail.entity_b.as_ref().map(EntityDto::from),
            identity_a: IdentityDto::from(&detail.identity_a),
            identity_b: IdentityDto::from(&detail.identity_b),
            decisions: detail.decisions.iter().map(DecisionDto::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DecisionResponseDto {
    decision: DecisionDto,
    candidate: CandidateDto,
    identity: IdentityDto,
    /// `true` when an identical earlier decision was returned.
    replayed: bool,
}

// --- parsing ---------------------------------------------------------------------------

fn service(state: &AppState) -> Result<&EntityResolutionService, ApiError> {
    state.entity_resolution.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "SERVICE_UNAVAILABLE",
            "Entity resolution is not available",
        )
    })
}

fn parse_id<T: std::str::FromStr>(
    raw: &str,
    code: &'static str,
    what: &str,
) -> Result<T, ApiError> {
    raw.trim()
        .parse()
        .map_err(|_| ApiError::invalid_id(code, what))
}

fn graph_and<T: std::str::FromStr>(
    path: Result<Path<(String, String)>, PathRejection>,
    code: &'static str,
    what: &str,
) -> Result<(GraphId, T), ApiError> {
    let Path((graph, other)) = path.map_err(|_| ApiError::invalid_graph_id())?;
    let graph = graph.parse().map_err(|_| ApiError::invalid_graph_id())?;
    Ok((graph, parse_id(&other, code, what)?))
}

/// A trimmed, non-empty query value.
fn optional(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}

fn invalid_query(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "INVALID_QUERY", message)
}

// --- handlers --------------------------------------------------------------------------

/// `POST /api/v1/graphs/{graph_id}/resolution/scans`
pub(crate) async fn create_scan(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<ScanBody>, JsonRejection>,
) -> Result<(StatusCode, HeaderMap, Json<ScanDto>), ApiError> {
    let graph_id = crate::routes::graph_id(path)?;
    let Json(body) = body?;
    let request = ScanRequest {
        entity_type: body.entity_type,
        max_entities: body.max_entities,
        rules: body
            .rules
            .into_iter()
            .map(|rule| RuleSpec {
                property: rule.property,
                kind: rule.kind,
                threshold: rule.threshold,
                country: rule.country,
                case_insensitive_local_part: rule.case_insensitive_local_part,
            })
            .collect(),
    };
    let scan = service(&state)?.scan(graph_id, request).await?;
    let mut headers = HeaderMap::new();
    if let Ok(location) = HeaderValue::from_str(&format!(
        "/api/v1/graphs/{graph_id}/resolution/scans/{}",
        scan.id
    )) {
        headers.insert(header::LOCATION, location);
    }
    Ok((StatusCode::CREATED, headers, Json(ScanDto::from(&scan))))
}

/// `GET /api/v1/graphs/{graph_id}/resolution/scans`
pub(crate) async fn list_scans(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    params: Result<Query<ScanListParams>, QueryRejection>,
) -> Result<Json<ScanListDto>, ApiError> {
    let graph_id = crate::routes::graph_id(path)?;
    let Query(params) = params?;
    let scans = service(&state)?.list_scans(graph_id, params.limit).await?;
    Ok(Json(ScanListDto {
        data: scans.iter().map(ScanDto::from).collect(),
    }))
}

/// `GET /api/v1/graphs/{graph_id}/resolution/scans/{scan_id}`
pub(crate) async fn get_scan(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<Json<ScanDto>, ApiError> {
    let (graph_id, scan_id): (GraphId, ResolutionScanId) =
        graph_and(path, "INVALID_SCAN_ID", "scan id")?;
    let scan = service(&state)?.get_scan(graph_id, scan_id).await?;
    Ok(Json(ScanDto::from(&scan)))
}

/// `GET /api/v1/graphs/{graph_id}/resolution/candidates`
pub(crate) async fn list_candidates(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    params: Result<Query<CandidateParams>, QueryRejection>,
) -> Result<Json<CandidateListDto>, ApiError> {
    let graph_id = crate::routes::graph_id(path)?;
    let Query(params) = params?;
    let status =
        match params.status.as_deref().map(str::trim) {
            None | Some("" | "all") => None,
            Some(raw) => Some(CandidateStatus::parse(raw).ok_or_else(|| {
                invalid_query("status must be pending, accepted, rejected or all")
            })?),
        };
    let scan_id = optional(params.scan_id.as_deref())
        .map(|raw| parse_id::<ResolutionScanId>(raw, "INVALID_SCAN_ID", "scan id"))
        .transpose()?;
    let entity_id = optional(params.entity_id.as_deref())
        .map(|raw| parse_id::<EntityId>(raw, "INVALID_ENTITY_ID", "entity id"))
        .transpose()?;
    let after = optional(params.cursor.as_deref())
        .map(|raw| {
            raw.parse::<ResolutionCandidateId>().map_err(|_| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "INVALID_CURSOR",
                    "The cursor is not valid",
                )
            })
        })
        .transpose()?;
    let limit = params.limit.unwrap_or(DEFAULT_CANDIDATE_PAGE);
    let page = service(&state)?
        .list_candidates(
            graph_id,
            CandidateQuery {
                status,
                scan_id,
                entity_id,
                limit,
                after,
            },
        )
        .await?;
    Ok(Json(CandidateListDto {
        data: page.candidates.iter().map(CandidateDto::from).collect(),
        next_cursor: page.next.map(|id| id.to_string()),
        limit,
        counts: CountsDto {
            pending: page.counts.pending,
            accepted: page.counts.accepted,
            rejected: page.counts.rejected,
        },
    }))
}

/// `GET /api/v1/graphs/{graph_id}/resolution/candidates/{candidate_id}`
pub(crate) async fn get_candidate(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<Json<CandidateDetailDto>, ApiError> {
    let (graph_id, candidate_id): (GraphId, ResolutionCandidateId) =
        graph_and(path, "INVALID_CANDIDATE_ID", "candidate id")?;
    let detail = service(&state)?
        .get_candidate(graph_id, candidate_id)
        .await?;
    Ok(Json(CandidateDetailDto::from(&detail)))
}

/// `POST /api/v1/graphs/{graph_id}/resolution/candidates/{candidate_id}/decisions`
pub(crate) async fn decide(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
    body: Result<Json<DecisionBody>, JsonRejection>,
) -> Result<(StatusCode, Json<DecisionResponseDto>), ApiError> {
    let (graph_id, candidate_id): (GraphId, ResolutionCandidateId) =
        graph_and(path, "INVALID_CANDIDATE_ID", "candidate id")?;
    let Json(body) = body?;
    let kind = DecisionKind::parse(&body.decision).ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "INVALID_DECISION",
            "decision must be same_entity or different_entities",
        )
    })?;
    let canonical_entity_id = body
        .canonical_entity_id
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| parse_id::<EntityId>(raw, "INVALID_ENTITY_ID", "canonical entity id"))
        .transpose()?;
    let result = service(&state)?
        .decide(
            graph_id,
            candidate_id,
            DecisionRequest {
                kind,
                canonical_entity_id,
                reconsider: body.reconsider,
            },
        )
        .await?;
    let status = if result.outcome.replayed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(DecisionResponseDto {
            decision: DecisionDto::from(&result.outcome.decision),
            candidate: CandidateDto::from(&result.outcome.candidate),
            identity: IdentityDto::from(&result.identity),
            replayed: result.outcome.replayed,
        }),
    ))
}

/// `GET /api/v1/graphs/{graph_id}/resolution/entities/{entity_id}`
pub(crate) async fn get_identity(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<Json<IdentityDto>, ApiError> {
    let (graph_id, entity_id): (GraphId, EntityId) =
        graph_and(path, "INVALID_ENTITY_ID", "entity id")?;
    let identity = service(&state)?.identity(graph_id, entity_id).await?;
    Ok(Json(IdentityDto::from(&identity)))
}
