//! Per-request context: request ids, tracing spans and the error envelope.

use std::time::Instant;

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use serde_json::json;
use tracing::{Instrument, info, info_span};
use uuid::Uuid;

use crate::error::{ErrorDetails, default_details};

/// The header carrying the request id, in both directions.
pub(crate) const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// The request id, available to handlers as a request extension (for
/// example to correlate calls to the AI service).
#[derive(Debug, Clone)]
pub(crate) struct RequestId(pub(crate) String);

/// Longest incoming request id that is accepted as-is.
const MAX_REQUEST_ID_LEN: usize = 128;

/// Accepts a client-supplied id if it is short, visible ASCII. Anything else
/// is replaced, so ids are always safe to log and echo.
fn incoming_request_id(request: &Request) -> Option<String> {
    request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|id| {
            !id.is_empty()
                && id.len() <= MAX_REQUEST_ID_LEN
                && id.bytes().all(|byte| byte.is_ascii_graphic())
        })
        .map(str::to_owned)
}

/// Middleware: assigns the request id, runs the request in a tracing span,
/// renders error envelopes and sets the `x-request-id` response header.
pub(crate) async fn request_context(mut request: Request, next: Next) -> Response {
    let request_id = incoming_request_id(&request).unwrap_or_else(|| Uuid::now_v7().to_string());
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));
    let span = info_span!(
        "http_request",
        request_id = %request_id,
        method = %request.method(),
        path = %request.uri().path(),
    );
    let started = Instant::now();
    let response = next.run(request).instrument(span.clone()).await;
    let mut response = render_error_envelope(response, &request_id);
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    span.in_scope(|| {
        info!(
            status = response.status().as_u16(),
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "request completed"
        );
    });
    response
}

/// Replaces the body of every error response with the stable envelope.
fn render_error_envelope(response: Response, request_id: &str) -> Response {
    let status = response.status();
    let details = response
        .extensions()
        .get::<ErrorDetails>()
        .cloned()
        .or_else(|| {
            (status.is_client_error() || status.is_server_error()).then(|| default_details(status))
        });
    let Some(details) = details else {
        return response;
    };
    let body = json!({
        "error": {
            "code": details.code,
            "message": details.message,
            "request_id": request_id,
        }
    });
    let (mut parts, _) = response.into_parts();
    parts
        .headers
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    parts.headers.remove(CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(body.to_string()))
}
