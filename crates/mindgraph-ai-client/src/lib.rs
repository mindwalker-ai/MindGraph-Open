//! HTTP adapter from MindGraph Core to the internal MindGraph AI service
//! (ADR-007, M5).
//!
//! [`HttpGraphBuilderAi`] implements the `GraphBuilderAi` port of
//! `mindgraph-core`:
//!
//! ```text
//! GraphBuilderService ──► HttpGraphBuilderAi ──HTTP──► mindgraph-ai (FastAPI)
//!                          GET  /ready                    └─► LLM provider
//!                          POST /internal/v1/graph-builder/suggest
//! ```
//!
//! The AI service is internal: it is reached over the Docker network and is
//! never published. Requests carry only the bounded dataset profile and the
//! active ontology. Responses are decoded into explicit DTOs; anything else
//! is an invalid response. `reqwest` types never leave this crate.

#![warn(missing_docs)]

use std::time::Duration;

use async_trait::async_trait;
use mindgraph_core::{
    AiError, AiMetadata, AiStatus, AiSuggestion, GraphBuilderAi, GraphBuilderRequest,
    GraphBuilderSuggestion, OntologyContext, RepairContext,
};
use mindgraph_ingestion::DatasetProfile;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// The default time allowed for one suggestion call. It covers the AI
/// service's own provider timeout and bounded retries.
pub const DEFAULT_SUGGEST_TIMEOUT: Duration = Duration::from_secs(300);
/// The time allowed for a readiness check.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
/// The largest accepted response body. The AI service bounds suggestions to
/// 256 KiB; this leaves room for the envelope.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const REQUEST_ID_HEADER: &str = "x-request-id";

/// An invalid AI service URL. The message never repeats the URL.
#[derive(Debug)]
pub struct InvalidAiUrl(&'static str);

impl std::fmt::Display for InvalidAiUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid AI service URL: {}", self.0)
    }
}

impl std::error::Error for InvalidAiUrl {}

/// The `GraphBuilderAi` port over HTTP. Cheap to clone; reuses one
/// connection pool.
#[derive(Debug, Clone)]
pub struct HttpGraphBuilderAi {
    client: reqwest::Client,
    suggest_url: Url,
    ready_url: Url,
}

impl HttpGraphBuilderAi {
    /// A client for the AI service at `base_url` (for example
    /// `http://ai:8000`).
    ///
    /// # Errors
    ///
    /// [`InvalidAiUrl`] unless the URL is plain `http` with a host and no
    /// credentials, query or fragment (the service is internal).
    pub fn new(base_url: &str, timeout: Duration) -> Result<Self, InvalidAiUrl> {
        let mut base = Url::parse(base_url.trim()).map_err(|_| InvalidAiUrl("not a URL"))?;
        if base.scheme() != "http" {
            return Err(InvalidAiUrl(
                "only http:// is supported for the internal service",
            ));
        }
        if base.host_str().is_none() || base.cannot_be_a_base() {
            return Err(InvalidAiUrl("a host is required"));
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(InvalidAiUrl("credentials are not allowed"));
        }
        if base.query().is_some() || base.fragment().is_some() {
            return Err(InvalidAiUrl("query strings and fragments are not allowed"));
        }
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let join = |path| base.join(path).map_err(|_| InvalidAiUrl("not a URL"));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|_| InvalidAiUrl("the HTTP client could not be created"))?;
        Ok(Self {
            client,
            suggest_url: join("internal/v1/graph-builder/suggest")?,
            ready_url: join("ready")?,
        })
    }
}

/// `POST /internal/v1/graph-builder/suggest` request body.
#[derive(Debug, Serialize)]
struct SuggestRequestDto<'a> {
    dataset_profile: &'a DatasetProfile,
    active_ontology: &'a OntologyContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    repair: Option<&'a RepairContext>,
}

/// `200 OK` of the suggest endpoint.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuggestResponseDto {
    suggestion: GraphBuilderSuggestion,
    metadata: MetadataDto,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataDto {
    prompt_version: String,
    provider: String,
    model: String,
}

/// `GET /ready`.
#[derive(Debug, Deserialize)]
struct ReadyDto {
    status: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// The AI service's error envelope.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorDto,
}

#[derive(Debug, Deserialize)]
struct ErrorDto {
    code: String,
}

fn transport_failure(err: &reqwest::Error) -> AiError {
    if err.is_timeout() {
        AiError::Timeout
    } else {
        AiError::Unavailable("the AI service cannot be reached".to_owned())
    }
}

/// Maps an error status of the AI service to an [`AiError`]. Only the code
/// and status are used: messages stay inside the AI service.
fn status_failure(status: StatusCode, body: &[u8]) -> AiError {
    let code = serde_json::from_slice::<ErrorEnvelope>(body)
        .map(|envelope| envelope.error.code)
        .unwrap_or_default();
    let detail = format!("{code} (HTTP {})", status.as_u16());
    match (status, code.as_str()) {
        (_, "AI_NOT_CONFIGURED") => AiError::NotConfigured,
        (StatusCode::TOO_MANY_REQUESTS, _) | (_, "PROVIDER_RATE_LIMITED") => AiError::RateLimited,
        (StatusCode::GATEWAY_TIMEOUT, _) | (_, "PROVIDER_TIMEOUT") => AiError::Timeout,
        (_, "PROVIDER_REJECTED") => AiError::ProviderRejected(detail),
        (_, "INVALID_PROVIDER_OUTPUT") => AiError::InvalidResponse(detail),
        (StatusCode::SERVICE_UNAVAILABLE | StatusCode::BAD_GATEWAY, _) => {
            AiError::Unavailable(detail)
        }
        _ => AiError::Internal(format!("the AI service failed: {detail}")),
    }
}

/// Reads a body of at most [`MAX_RESPONSE_BYTES`].
async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, AiError> {
    let too_large = || AiError::InvalidResponse("the AI service response is too large".to_owned());
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| transport_failure(&err))?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[async_trait]
impl GraphBuilderAi for HttpGraphBuilderAi {
    async fn status(&self) -> AiStatus {
        let response = match self
            .client
            .get(self.ready_url.clone())
            .timeout(STATUS_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                warn!(
                    timeout = err.is_timeout(),
                    "AI service readiness check failed"
                );
                return AiStatus::Unavailable;
            }
        };
        let status = response.status();
        let Ok(body) = bounded_body(response).await else {
            return AiStatus::Unavailable;
        };
        match serde_json::from_slice::<ReadyDto>(&body) {
            Ok(ready) if status.is_success() && ready.status == "ready" => AiStatus::Ready {
                provider: ready.provider.unwrap_or_default(),
                model: ready.model.unwrap_or_default(),
            },
            Ok(ready) if ready.status == "not_configured" => AiStatus::NotConfigured,
            _ => AiStatus::Unavailable,
        }
    }

    async fn suggest(&self, request: &GraphBuilderRequest) -> Result<AiSuggestion, AiError> {
        let body = SuggestRequestDto {
            dataset_profile: &request.dataset_profile,
            active_ontology: &request.active_ontology,
            repair: request.repair.as_ref(),
        };
        let response = self
            .client
            .post(self.suggest_url.clone())
            .header(REQUEST_ID_HEADER, &request.correlation_id)
            .json(&body)
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
        let decoded: SuggestResponseDto = serde_json::from_slice(&bytes).map_err(|_| {
            AiError::InvalidResponse("the AI service response is not a valid suggestion".to_owned())
        })?;
        Ok(AiSuggestion {
            suggestion: decoded.suggestion,
            metadata: AiMetadata {
                prompt_version: decoded.metadata.prompt_version,
                provider: decoded.metadata.provider,
                model: decoded.metadata.model,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_validated_without_echoing_them() {
        for bad in [
            "ftp://ai:8000",
            "https://ai:8000",
            "http://user:secret@ai:8000",
            "http://ai:8000/?token=x",
            "not a url",
        ] {
            let err = HttpGraphBuilderAi::new(bad, DEFAULT_SUGGEST_TIMEOUT).unwrap_err();
            assert!(!err.to_string().contains("secret"), "{err}");
        }
        let client = HttpGraphBuilderAi::new("http://ai:8000", DEFAULT_SUGGEST_TIMEOUT).unwrap();
        assert_eq!(
            client.suggest_url.as_str(),
            "http://ai:8000/internal/v1/graph-builder/suggest"
        );
        let nested = HttpGraphBuilderAi::new("http://gw/ai", DEFAULT_SUGGEST_TIMEOUT).unwrap();
        assert_eq!(nested.ready_url.as_str(), "http://gw/ai/ready");
    }

    #[test]
    fn error_statuses_map_to_categories() {
        let envelope = |code: &str| format!(r#"{{"error":{{"code":"{code}","message":"x"}}}}"#);
        let cases = [
            (503, envelope("AI_NOT_CONFIGURED"), "AI_NOT_CONFIGURED"),
            (503, envelope("PROVIDER_UNAVAILABLE"), "AI_UNAVAILABLE"),
            (429, envelope("PROVIDER_RATE_LIMITED"), "AI_RATE_LIMITED"),
            (504, envelope("PROVIDER_TIMEOUT"), "AI_TIMEOUT"),
            (502, envelope("PROVIDER_REJECTED"), "AI_PROVIDER_REJECTED"),
            (
                502,
                envelope("INVALID_PROVIDER_OUTPUT"),
                "AI_INVALID_RESPONSE",
            ),
            (422, envelope("INVALID_REQUEST"), "AI_INTERNAL_ERROR"),
            (500, "oops".to_owned(), "AI_INTERNAL_ERROR"),
        ];
        for (status, body, code) in cases {
            let error = status_failure(StatusCode::from_u16(status).unwrap(), body.as_bytes());
            assert_eq!(error.code(), code, "{status} {body}");
        }
    }
}
