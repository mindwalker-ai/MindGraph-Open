//! The `GraphRagAi` port over HTTP (M7).
//!
//! ```text
//! GraphRagService ──► HttpGraphRagAi ──HTTP──► mindgraph-ai (FastAPI)
//!                      GET  /ready
//!                      POST /internal/v1/graphrag/interpret   question + ontology + selected entities
//!                                                             (+ earlier questions and their entities)
//!                      POST /internal/v1/graphrag/synthesize  question + bounded evidence context
//!                                                             (+ earlier questions)
//! ```
//!
//! Requests carry only what the port hands over; responses are decoded
//! into explicit DTOs (unknown fields rejected) and stay untrusted: the
//! intent is validated and the citations are checked by MindGraph Core.
//! The conversation fields of a follow-up are omitted for a first question.

use std::time::Duration;

use async_trait::async_trait;
use mindgraph_core::{
    AiError, AiMetadata, AiStatus, ConversationTurnContext, GraphRagAi, Interpretation,
    InterpretationLimits, InterpretationRequest, KnownEntityContext, OntologyContext,
    RetrievalCompleteness, Synthesis, SynthesisRequest, SynthesizedAnswer,
};
use mindgraph_graphrag::RetrievalIntent;
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::{
    InvalidAiUrl, MetadataDto, REQUEST_ID_HEADER, base_url_of, bounded_body, http_client,
    readiness, status_failure, transport_failure,
};

/// The default time allowed for one GraphRAG call. MindGraph Core's own
/// per-question deadline is usually shorter and wins.
pub const DEFAULT_GRAPHRAG_HTTP_TIMEOUT: Duration = Duration::from_secs(120);

/// The `GraphRagAi` port over HTTP. Cheap to clone; reuses one connection
/// pool.
#[derive(Debug, Clone)]
pub struct HttpGraphRagAi {
    client: reqwest::Client,
    interpret_url: Url,
    synthesize_url: Url,
    ready_url: Url,
}

impl HttpGraphRagAi {
    /// A client for the AI service at `base_url` (for example
    /// `http://ai:8000`).
    ///
    /// # Errors
    ///
    /// [`InvalidAiUrl`] unless the URL is plain `http` with a host and no
    /// credentials, query or fragment (the service is internal).
    pub fn new(base_url: &str, timeout: Duration) -> Result<Self, InvalidAiUrl> {
        let base = base_url_of(base_url)?;
        let join = |path| base.join(path).map_err(|_| InvalidAiUrl("not a URL"));
        Ok(Self {
            client: http_client(timeout)?,
            interpret_url: join("internal/v1/graphrag/interpret")?,
            synthesize_url: join("internal/v1/graphrag/synthesize")?,
            ready_url: join("ready")?,
        })
    }

    async fn post<B: Serialize + Sync, R: DeserializeOwned>(
        &self,
        url: &Url,
        correlation_id: &str,
        body: &B,
    ) -> Result<R, AiError> {
        let response = self
            .client
            .post(url.clone())
            .header(REQUEST_ID_HEADER, correlation_id)
            .json(body)
            .send()
            .await
            .map_err(|err| transport_failure(&err))?;
        let status = response.status();
        let bytes = bounded_body(response).await?;
        if !status.is_success() {
            let error = status_failure(status, &bytes);
            debug!(
                status = status.as_u16(),
                code = error.code(),
                "AI service returned an error"
            );
            return Err(error);
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            AiError::InvalidResponse("the AI service response does not match the schema".to_owned())
        })
    }
}

/// `POST /internal/v1/graphrag/interpret` request body.
#[derive(Debug, Serialize)]
struct InterpretRequestDto<'a> {
    question: &'a str,
    ontology: &'a OntologyContext,
    selected_entities: &'a [KnownEntityContext],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    conversation: &'a [ConversationTurnContext],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    earlier_entities: &'a [KnownEntityContext],
    limits: InterpretationLimits,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InterpretResponseDto {
    intent: RetrievalIntent,
    metadata: MetadataDto,
}

/// `POST /internal/v1/graphrag/synthesize` request body.
#[derive(Debug, Serialize)]
struct SynthesizeRequestDto<'a> {
    question: &'a str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    earlier_questions: &'a [String],
    context: &'a Value,
    retrieval: &'a RetrievalCompleteness,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SynthesizeResponseDto {
    answer: SynthesizedAnswer,
    metadata: MetadataDto,
}

fn metadata(dto: MetadataDto) -> AiMetadata {
    AiMetadata {
        prompt_version: dto.prompt_version,
        provider: dto.provider,
        model: dto.model,
    }
}

#[async_trait]
impl GraphRagAi for HttpGraphRagAi {
    async fn status(&self) -> AiStatus {
        readiness(&self.client, &self.ready_url).await
    }

    async fn interpret(&self, request: &InterpretationRequest) -> Result<Interpretation, AiError> {
        let body = InterpretRequestDto {
            question: &request.question,
            ontology: &request.ontology,
            selected_entities: &request.selected_entities,
            conversation: &request.conversation,
            earlier_entities: &request.earlier_entities,
            limits: request.limits,
        };
        let decoded: InterpretResponseDto = self
            .post(&self.interpret_url, &request.correlation_id, &body)
            .await?;
        Ok(Interpretation {
            intent: decoded.intent,
            metadata: metadata(decoded.metadata),
        })
    }

    async fn synthesize(&self, request: &SynthesisRequest) -> Result<Synthesis, AiError> {
        let body = SynthesizeRequestDto {
            question: &request.question,
            earlier_questions: &request.earlier_questions,
            context: &request.context,
            retrieval: &request.retrieval,
        };
        let decoded: SynthesizeResponseDto = self
            .post(&self.synthesize_url, &request.correlation_id, &body)
            .await?;
        Ok(Synthesis {
            answer: decoded.answer,
            metadata: metadata(decoded.metadata),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_joined_to_the_base_url() {
        let client = HttpGraphRagAi::new("http://ai:8000", DEFAULT_GRAPHRAG_HTTP_TIMEOUT).unwrap();
        assert_eq!(
            client.interpret_url.as_str(),
            "http://ai:8000/internal/v1/graphrag/interpret"
        );
        assert_eq!(
            client.synthesize_url.as_str(),
            "http://ai:8000/internal/v1/graphrag/synthesize"
        );
        assert!(HttpGraphRagAi::new("https://ai", DEFAULT_GRAPHRAG_HTTP_TIMEOUT).is_err());
    }
}
