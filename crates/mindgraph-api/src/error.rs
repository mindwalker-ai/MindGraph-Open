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
use mindgraph_core::ApplicationError;
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
            ApplicationError::Conflict(message) => {
                warn!(error = %message, "request conflicted with existing data");
                Self::new(
                    StatusCode::CONFLICT,
                    "CONFLICT",
                    "The request conflicts with existing data",
                )
            }
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

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        match rejection {
            JsonRejection::MissingJsonContentType(_) => Self::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "UNSUPPORTED_MEDIA_TYPE",
                "Expected `Content-Type: application/json`",
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
