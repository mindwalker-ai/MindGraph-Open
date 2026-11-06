//! API errors and their mapping from application errors (ADR-006).
//!
//! Handlers return [`ApiError`]. Its response carries the code and message
//! as an extension; the request-context middleware renders the stable
//! envelope, adding the request id:
//!
//! ```json
//! {"error": {"code": "GRAPH_NOT_FOUND", "message": "Graph was not found", "request_id": "…"}}
//! ```

use axum::Extension;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use mindgraph_core::{ApplicationError, GraphRagError, ResolutionError};
use tracing::{error, warn};

/// The code and message of an error response, rendered into the envelope by
/// the request-context middleware.
#[derive(Debug, Clone)]
pub(crate) struct ErrorDetails {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// An error returned by a handler.
#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    details: ErrorDetails,
}

impl ApiError {
    /// The stable error code.
    #[cfg(test)]
    pub(crate) const fn code(&self) -> &'static str {
        self.details.code
    }

    pub(crate) fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            details: ErrorDetails {
                code,
                message: message.into(),
            },
        }
    }

    pub(crate) fn invalid_graph_id() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_GRAPH_ID",
            "The graph id must be a MindGraph id (UUIDv7)",
        )
    }

    pub(crate) fn invalid_id(code: &'static str, what: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            code,
            format!("The {what} must be a MindGraph id (UUIDv7)"),
        )
    }

    pub(crate) fn not_ready() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "NOT_READY",
            "The metadata database is not reachable",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Extension(self.details)).into_response()
    }
}

impl From<ApplicationError> for ApiError {
    fn from(err: ApplicationError) -> Self {
        match err {
            ApplicationError::Validation(domain) => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "VALIDATION_FAILED",
                domain.to_string(),
            ),
            ApplicationError::InvalidRequest(message) => {
                Self::new(StatusCode::BAD_REQUEST, "INVALID_REQUEST", message)
            }
            ApplicationError::GraphNotFound(_) => Self::new(
                StatusCode::NOT_FOUND,
                "GRAPH_NOT_FOUND",
                "Graph was not found",
            ),
            ApplicationError::DataSourceNotFound(_) => Self::new(
                StatusCode::NOT_FOUND,
                "DATA_SOURCE_NOT_FOUND",
                "Data source was not found",
            ),
            ApplicationError::JobNotFound(_) => {
                Self::new(StatusCode::NOT_FOUND, "JOB_NOT_FOUND", "Job was not found")
            }
            ApplicationError::EntityNotFound(_) => Self::new(
                StatusCode::NOT_FOUND,
                "ENTITY_NOT_FOUND",
                "Entity was not found",
            ),
            ApplicationError::Conflict(message) => {
                // Service conflict messages describe state, never stored data.
                warn!(error = %message, "request conflicted with existing data");
                Self::new(StatusCode::CONFLICT, "CONFLICT", message)
            }
            ApplicationError::UnsupportedFormat(message) => Self::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "UNSUPPORTED_FILE_TYPE",
                message,
            ),
            ApplicationError::PayloadTooLarge { max_bytes } => Self::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "PAYLOAD_TOO_LARGE",
                format!("The upload exceeds the maximum size of {max_bytes} bytes"),
            ),
            ApplicationError::Unprocessable { code, message } => {
                Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
            }
            ApplicationError::AiNotConfigured => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AI_NOT_CONFIGURED",
                "The AI Graph Builder is not configured; build the ontology and mapping manually",
            ),
            ApplicationError::AiUnavailable => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AI_UNAVAILABLE",
                "The AI service is unavailable; try again later or continue manually",
            ),
            ApplicationError::GraphRag(error) => Self::from(error),
            ApplicationError::Resolution(error) => Self::from(error),
            ApplicationError::Unavailable(source) => {
                warn!(error = %source, "a required backend is unavailable");
                Self::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "SERVICE_UNAVAILABLE",
                    "A required backend is unavailable; try again later",
                )
            }
            ApplicationError::Internal(source) => {
                // Details go to the logs, never to the client.
                error!(error = %source, "request failed unexpectedly");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "An unexpected error occurred",
                )
            }
        }
    }
}

impl From<GraphRagError> for ApiError {
    fn from(error: GraphRagError) -> Self {
        match error {
            GraphRagError::InvalidQuestion(message) => {
                Self::new(StatusCode::BAD_REQUEST, "INVALID_QUESTION", message)
            }
            GraphRagError::InvalidHistory(message) => {
                Self::new(StatusCode::BAD_REQUEST, "INVALID_HISTORY", message)
            }
            GraphRagError::InvalidScope(message) => {
                Self::new(StatusCode::BAD_REQUEST, "INVALID_SCOPE", message)
            }
            // Messages name schema names and limits, never stored values.
            GraphRagError::InvalidRetrievalPlan(message) => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_RETRIEVAL_PLAN",
                format!("The question could not be turned into a valid graph retrieval: {message}"),
            ),
            GraphRagError::RetrievalLimitExceeded(message) => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "RETRIEVAL_LIMIT_EXCEEDED",
                format!("The retrieval would exceed MindGraph's limits: {message}"),
            ),
            GraphRagError::AiNotConfigured => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AI_NOT_CONFIGURED",
                "Ask MindGraph needs a model: set MINDGRAPH_LLM_PROVIDER (see .env.example). Graph exploration keeps working without it",
            ),
            GraphRagError::AiUnavailable => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AI_UNAVAILABLE",
                "The AI service or model provider is unavailable; try again later",
            ),
            GraphRagError::AiTimeout => Self::new(
                StatusCode::GATEWAY_TIMEOUT,
                "AI_TIMEOUT",
                "The model provider did not answer in time; try again or ask a narrower question",
            ),
            GraphRagError::Timeout(seconds) => Self::new(
                StatusCode::GATEWAY_TIMEOUT,
                "AI_TIMEOUT",
                format!(
                    "The question was not answered within {seconds} seconds; try again or ask a narrower question"
                ),
            ),
            GraphRagError::AiInvalidResponse => Self::new(
                StatusCode::BAD_GATEWAY,
                "AI_INVALID_RESPONSE",
                "The model did not return a valid answer; try again",
            ),
        }
    }
}

impl From<ResolutionError> for ApiError {
    fn from(error: ResolutionError) -> Self {
        // Messages name entity types, properties, rule kinds, ids and limits;
        // never property values.
        let message = error.to_string();
        match error {
            ResolutionError::UnknownEntityType(_) => {
                Self::new(StatusCode::BAD_REQUEST, "UNKNOWN_ENTITY_TYPE", message)
            }
            ResolutionError::InvalidRule(_) => {
                Self::new(StatusCode::BAD_REQUEST, "INVALID_RULE", message)
            }
            ResolutionError::ScanNotFound(_) => Self::new(
                StatusCode::NOT_FOUND,
                "SCAN_NOT_FOUND",
                "Resolution scan was not found",
            ),
            ResolutionError::CandidateNotFound(_) => Self::new(
                StatusCode::NOT_FOUND,
                "CANDIDATE_NOT_FOUND",
                "Resolution candidate was not found",
            ),
            ResolutionError::InvalidDecision(_) => {
                Self::new(StatusCode::BAD_REQUEST, "INVALID_DECISION", message)
            }
            ResolutionError::DecisionConflict(_) => {
                Self::new(StatusCode::CONFLICT, "DECISION_CONFLICT", message)
            }
            ResolutionError::IdentityConflict(_) => {
                Self::new(StatusCode::CONFLICT, "IDENTITY_CONFLICT", message)
            }
            ResolutionError::EntityUnavailable(_) => {
                Self::new(StatusCode::CONFLICT, "ENTITY_UNAVAILABLE", message)
            }
            ResolutionError::InvalidCursor => {
                Self::new(StatusCode::BAD_REQUEST, "INVALID_CURSOR", message)
            }
            ResolutionError::ScanTimeout(_) => Self::new(
                StatusCode::GATEWAY_TIMEOUT,
                "RESOLUTION_SCAN_TIMEOUT",
                format!("{message}; scan fewer entities or use fewer rules"),
            ),
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        match rejection {
            JsonRejection::MissingJsonContentType(_) => Self::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "UNSUPPORTED_MEDIA_TYPE",
                "Expected `Content-Type: application/json`",
            ),
            // A body over the route's limit is not malformed JSON.
            other if other.status() == StatusCode::PAYLOAD_TOO_LARGE => Self::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "PAYLOAD_TOO_LARGE",
                "The request body is too large",
            ),
            other => Self::new(StatusCode::BAD_REQUEST, "INVALID_JSON", other.body_text()),
        }
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_QUERY",
            rejection.body_text(),
        )
    }
}

impl From<PathRejection> for ApiError {
    fn from(_: PathRejection) -> Self {
        Self::invalid_graph_id()
    }
}

/// The envelope fields for an error response produced outside our handlers,
/// for example Axum's own 404 or 405.
pub(crate) fn default_details(status: StatusCode) -> ErrorDetails {
    let (code, message) = match status {
        StatusCode::NOT_FOUND => ("ROUTE_NOT_FOUND", "No route matches this path"),
        StatusCode::METHOD_NOT_ALLOWED => (
            "METHOD_NOT_ALLOWED",
            "This method is not allowed on this path",
        ),
        StatusCode::PAYLOAD_TOO_LARGE => ("PAYLOAD_TOO_LARGE", "The request body is too large"),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => {
            ("UNSUPPORTED_MEDIA_TYPE", "Unsupported request content type")
        }
        status if status.is_server_error() => ("INTERNAL_ERROR", "An unexpected error occurred"),
        _ => ("BAD_REQUEST", "The request could not be processed"),
    };
    ErrorDetails {
        code,
        message: message.to_owned(),
    }
}
