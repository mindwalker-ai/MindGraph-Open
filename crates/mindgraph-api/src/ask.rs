//! GraphRAG endpoints (M7): `POST /api/v1/graphs/{graph_id}/ask` and
//! `GET /api/v1/graphrag/status`.
//!
//! Handlers parse the request, call `GraphRagService` and render explicit
//! DTOs. The request accepts a question, a scope and, for a follow-up, up
//! to three earlier questions with the entity ids they cited (the client
//! keeps the conversation; nothing is stored here), never a query
//! language. The response carries the checked answer, the full evidence
//! subgraph (the same entity and relationship shapes as the explorer, plus
//! their references), the valid citations and what retrieval covered. No
//! raw model output, prompt or provider detail is ever returned.

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use mindgraph_core::{
    AiStatus, AnswerMetadata, AnswerSource, AnswerStatus, AskQuestion, AskScope, ConversationTurn,
    GraphRagAnswer, GraphRagService, RetrievalReport,
};
use mindgraph_domain::EntityId;
use mindgraph_graphrag::{
    CitedObject, MAX_CONVERSATION_TURNS, MAX_SEED_ENTITIES, MAX_TURN_ENTITIES,
};
use serde::{Deserialize, Serialize};

use crate::dto::GraphBuilderStatusResponse;
use crate::error::ApiError;
use crate::explorer::{EntityDto, RelationshipDto};
use crate::request_context::RequestId;
use crate::routes::{AppState, graph_id};

/// `POST /api/v1/graphs/{graph_id}/ask`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AskRequest {
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    scope: Option<ScopeRequest>,
    #[serde(default)]
    history: Option<Vec<HistoryTurnRequest>>,
}

/// An earlier question of the conversation (follow-ups).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryTurnRequest {
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    entity_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScopeRequest {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    entity_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct EvidenceEntityDto {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(flatten)]
    entity: EntityDto,
}

#[derive(Debug, Serialize)]
pub(crate) struct EvidenceRelationshipDto {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(flatten)]
    relationship: RelationshipDto,
}

#[derive(Debug, Serialize)]
pub(crate) struct EvidencePathDto {
    #[serde(rename = "ref")]
    reference: String,
    entity_ids: Vec<String>,
    relationship_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct EvidenceDto {
    entities: Vec<EvidenceEntityDto>,
    relationships: Vec<EvidenceRelationshipDto>,
    paths: Vec<EvidencePathDto>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CitationDto {
    #[serde(rename = "ref")]
    reference: String,
    kind: &'static str,
    /// The cited entity or relationship id; absent for paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AskResponse {
    status: AnswerStatus,
    answer: String,
    answer_source: AnswerSource,
    evidence: EvidenceDto,
    citations: Vec<CitationDto>,
    warnings: Vec<String>,
    retrieval: RetrievalReport,
    metadata: AnswerMetadata,
}

impl From<GraphRagAnswer> for AskResponse {
    fn from(answer: GraphRagAnswer) -> Self {
        let evidence = &answer.evidence;
        let reference = |kind: char, index: usize| format!("{kind}{}", index + 1);
        Self {
            status: answer.status,
            answer: answer.answer,
            answer_source: answer.source,
            citations: answer
                .citations
                .iter()
                .map(|citation| CitationDto {
                    reference: citation.reference.to_string(),
                    kind: citation.reference.kind.as_str(),
                    id: match citation.object {
                        CitedObject::Entity(id) => Some(id.to_string()),
                        CitedObject::Relationship(id) => Some(id.to_string()),
                        CitedObject::Path(_) => None,
                    },
                })
                .collect(),
            evidence: EvidenceDto {
                entities: evidence
                    .entities()
                    .iter()
                    .enumerate()
                    .map(|(index, entity)| EvidenceEntityDto {
                        reference: reference('E', index),
                        entity: EntityDto::from(entity),
                    })
                    .collect(),
                relationships: evidence
                    .relationships()
                    .iter()
                    .enumerate()
                    .map(|(index, relationship)| EvidenceRelationshipDto {
                        reference: reference('R', index),
                        relationship: RelationshipDto::from(relationship),
                    })
                    .collect(),
                paths: evidence
                    .paths()
                    .iter()
                    .enumerate()
                    .map(|(index, path)| EvidencePathDto {
                        reference: reference('P', index),
                        entity_ids: path.entity_ids().iter().map(ToString::to_string).collect(),
                        relationship_ids: path
                            .relationship_ids()
                            .iter()
                            .map(ToString::to_string)
                            .collect(),
                    })
                    .collect(),
            },
            warnings: answer.warnings,
            retrieval: answer.retrieval,
            metadata: answer.metadata,
        }
    }
}

fn graphrag(state: &AppState) -> Result<&GraphRagService, ApiError> {
    state.graphrag.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "AI_NOT_CONFIGURED",
            "Ask MindGraph is not configured; graph exploration keeps working",
        )
    })
}

fn invalid_scope(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "INVALID_SCOPE", message)
}

fn scope(raw: Option<ScopeRequest>) -> Result<AskScope, ApiError> {
    let Some(raw) = raw else {
        return Ok(AskScope::BoundedGraph);
    };
    let ids = raw.entity_ids.unwrap_or_default();
    match raw.mode.as_deref().map(str::trim) {
        None | Some("bounded_graph") if ids.is_empty() => Ok(AskScope::BoundedGraph),
        None | Some("bounded_graph") => Err(invalid_scope(
            "entity_ids are only allowed with mode selected_entities",
        )),
        Some("selected_entities") => {
            if ids.is_empty() || ids.len() > MAX_SEED_ENTITIES {
                return Err(invalid_scope(format!(
                    "selected_entities needs 1 to {MAX_SEED_ENTITIES} entity ids"
                )));
            }
            ids.iter()
                .map(|raw| {
                    raw.trim()
                        .parse::<EntityId>()
                        .map_err(|_| invalid_scope("entity_ids must be MindGraph ids (UUIDv7)"))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(AskScope::SelectedEntities)
        }
        Some(_) => Err(invalid_scope(
            "scope.mode must be selected_entities or bounded_graph",
        )),
    }
}

fn invalid_history(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "INVALID_HISTORY", message)
}

/// The earlier questions: their count and ids are checked here, their text
/// by the service.
fn history(raw: Option<Vec<HistoryTurnRequest>>) -> Result<Vec<ConversationTurn>, ApiError> {
    let turns = raw.unwrap_or_default();
    if turns.len() > MAX_CONVERSATION_TURNS {
        return Err(invalid_history(format!(
            "history may carry at most {MAX_CONVERSATION_TURNS} earlier questions"
        )));
    }
    turns
        .into_iter()
        .map(|turn| {
            let ids = turn.entity_ids.unwrap_or_default();
            if ids.len() > MAX_TURN_ENTITIES {
                return Err(invalid_history(format!(
                    "an earlier question may carry at most {MAX_TURN_ENTITIES} entity ids"
                )));
            }
            let entity_ids = ids
                .iter()
                .map(|raw| {
                    raw.trim().parse::<EntityId>().map_err(|_| {
                        invalid_history("history entity_ids must be MindGraph ids (UUIDv7)")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ConversationTurn {
                question: turn.question.unwrap_or_default(),
                entity_ids,
            })
        })
        .collect()
}

/// `POST /api/v1/graphs/{graph_id}/ask`
pub(crate) async fn ask(
    State(state): State<AppState>,
    request_id: Option<Extension<RequestId>>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<AskRequest>, JsonRejection>,
) -> Result<Json<AskResponse>, ApiError> {
    let graph_id = graph_id(path)?;
    let Json(request) = body?;
    let question = request.question.unwrap_or_default();
    let scope = scope(request.scope)?;
    let history = history(request.history)?;
    let correlation_id = request_id.map_or_else(|| "-".to_owned(), |Extension(id)| id.0);
    let answer = graphrag(&state)?
        .ask(
            graph_id,
            AskQuestion {
                question,
                scope,
                history,
                correlation_id,
            },
        )
        .await?;
    Ok(Json(AskResponse::from(answer)))
}

/// `GET /api/v1/graphrag/status`: can questions be answered now? Never
/// names URLs or keys.
pub(crate) async fn status(State(state): State<AppState>) -> Json<GraphBuilderStatusResponse> {
    let status = match &state.graphrag {
        Some(graphrag) => graphrag.status().await,
        None => AiStatus::NotConfigured,
    };
    Json(GraphBuilderStatusResponse::from(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(mode: Option<&str>, ids: &[&str]) -> Option<ScopeRequest> {
        Some(ScopeRequest {
            mode: mode.map(str::to_owned),
            entity_ids: Some(ids.iter().map(|id| (*id).to_owned()).collect()),
        })
    }

    #[test]
    fn scopes_parse_strictly() {
        let id = EntityId::new().to_string();
        assert_eq!(scope(None).unwrap(), AskScope::BoundedGraph);
        assert_eq!(
            scope(raw(Some("bounded_graph"), &[])).unwrap(),
            AskScope::BoundedGraph
        );
        assert!(matches!(
            scope(raw(Some("selected_entities"), &[&id])).unwrap(),
            AskScope::SelectedEntities(ids) if ids.len() == 1
        ));
        for bad in [
            raw(Some("selected_entities"), &[]),
            raw(Some("selected_entities"), &["nope"]),
            raw(Some("bounded_graph"), &[&id]),
            raw(Some("everything"), &[]),
            raw(Some("selected_entities"), &[&id, &id, &id, &id, &id, &id]),
        ] {
            assert!(scope(bad).is_err());
        }
    }

    fn turn(question: &str, ids: &[&str]) -> HistoryTurnRequest {
        HistoryTurnRequest {
            question: Some(question.to_owned()),
            entity_ids: Some(ids.iter().map(|id| (*id).to_owned()).collect()),
        }
    }

    #[test]
    fn histories_parse_strictly() {
        let id = EntityId::new().to_string();
        assert!(history(None).unwrap().is_empty());
        let parsed = history(Some(vec![turn("Who?", &[&id])])).unwrap();
        assert_eq!(parsed[0].entity_ids[0].to_string(), id);
        let eleven: Vec<&str> = std::iter::repeat_n(id.as_str(), 11).collect();
        for bad in [
            vec![
                turn("a", &[]),
                turn("b", &[]),
                turn("c", &[]),
                turn("d", &[]),
            ],
            vec![turn("a", &["nope"])],
            vec![turn("a", &eleven)],
        ] {
            assert_eq!(history(Some(bad)).unwrap_err().code(), "INVALID_HISTORY");
        }
    }
}
