//! Machine-readable tool errors.
//!
//! A failed tool call is an MCP *tool execution error* (`isError: true`), so
//! the agent sees what went wrong and can correct its next call; it never
//! crashes the server or becomes a protocol error. The result carries one
//! JSON text block:
//!
//! ```json
//! {"error": {"code": "ENTITY_NOT_FOUND", "message": "…", "hint": "…",
//!            "http_status": 404, "request_id": "…"}}
//! ```
//!
//! Codes from MindGraph Core (`GRAPH_NOT_FOUND`, `ENTITY_NOT_FOUND`,
//! `INVALID_QUERY`, `AI_NOT_CONFIGURED`, …) pass through unchanged. The
//! server adds its own codes for problems Core cannot report, listed in
//! [`codes`]. Messages never contain credentials, stack traces or response
//! bodies that are not a Core error envelope.

use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock};
use serde::Serialize;
use serde_json::json;

/// Error codes produced by `mindgraph-mcp` itself.
pub mod codes {
    /// A tool argument is missing, malformed or out of bounds.
    pub const INVALID_ARGUMENT: &str = "INVALID_ARGUMENT";
    /// The result exceeds `MINDGRAPH_MCP_MAX_RESULT_BYTES`.
    pub const RESULT_TOO_LARGE: &str = "RESULT_TOO_LARGE";
    /// MindGraph Core cannot be reached (not started, wrong URL or port).
    pub const CORE_UNAVAILABLE: &str = "CORE_UNAVAILABLE";
    /// MindGraph Core did not answer within the configured deadline.
    pub const CORE_TIMEOUT: &str = "CORE_TIMEOUT";
    /// MindGraph Core answered with a redirect, which is never followed.
    pub const CORE_REDIRECT: &str = "CORE_REDIRECT";
    /// MindGraph Core answered with an error but no error envelope.
    pub const CORE_ERROR: &str = "CORE_ERROR";
    /// A successful Core response was not the expected JSON document.
    pub const INVALID_CORE_RESPONSE: &str = "INVALID_CORE_RESPONSE";
}

/// One failed tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolError {
    /// Stable, machine-readable code.
    pub code: String,
    /// What went wrong, for the agent and the user.
    pub message: String,
    /// What to try next, when there is a clear next step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// The HTTP status MindGraph Core answered with, if it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// The `x-request-id` of the Core request, to find it in Core's logs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl ToolError {
    /// A new error with a code and message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        let mut error = Self {
            code: code.into(),
            message: message.into(),
            hint: None,
            http_status: None,
            request_id: None,
        };
        error.hint = default_hint(&error.code).map(str::to_owned);
        error
    }

    /// An invalid tool argument.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID_ARGUMENT, message)
    }

    /// Replaces the hint.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Records the Core request this error belongs to.
    #[must_use]
    pub fn with_request(mut self, http_status: Option<u16>, request_id: &str) -> Self {
        self.http_status = http_status;
        self.request_id = Some(request_id.to_owned());
        self
    }

    /// The JSON document returned to the agent.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        json!({ "error": self })
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolError {}

/// A tool execution error with the error as JSON text. It deliberately has
/// no `structuredContent`: clients validate structured content against the
/// tool's output schema, which describes successful results only.
impl IntoCallToolResult for ToolError {
    fn into_call_tool_result(self) -> Result<CallToolResponse, rmcp::ErrorData> {
        Ok(CallToolResult::error(vec![ContentBlock::text(self.to_json().to_string())]).into())
    }
}

fn default_hint(code: &str) -> Option<&'static str> {
    Some(match code {
        codes::INVALID_ARGUMENT => "Check the tool's input schema and call it again.",
        codes::RESULT_TOO_LARGE => {
            "Narrow the request: lower `limit`, add a `relationship_type` or `property_filters`, or request fewer `entity_ids`."
        }
        codes::CORE_UNAVAILABLE => {
            "Start MindGraph (docker compose up) and check MINDGRAPH_MCP_CORE_URL; run `mindgraph-mcp --check` to test the configuration."
        }
        codes::CORE_TIMEOUT => {
            "Try a smaller request; MINDGRAPH_MCP_TIMEOUT_SECONDS sets the deadline."
        }
        "GRAPH_NOT_FOUND" => {
            "MINDGRAPH_MCP_GRAPH_ID names no graph project in this MindGraph Core."
        }
        "ENTITY_NOT_FOUND" => {
            "No entity with this id exists in the configured graph; find ids with search_entities."
        }
        "INVALID_QUERY" | "INVALID_REQUEST" => {
            "Call get_graph_schema for the valid entity types, relationship types and properties."
        }
        "AI_NOT_CONFIGURED" => {
            "ask_graph needs a model configured in MindGraph Core (MINDGRAPH_LLM_PROVIDER); the other tools work without one."
        }
        "AI_TIMEOUT" => {
            "The model did not answer in time; MINDGRAPH_MCP_ASK_TIMEOUT_SECONDS and MINDGRAPH_GRAPHRAG_TIMEOUT_SECONDS set the deadlines."
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_tool_execution_errors_with_json_text() {
        let error = ToolError::new("ENTITY_NOT_FOUND", "Entity not found")
            .with_request(Some(404), "019a5f6e-0000-7000-8000-000000000001");
        let response = error.clone().into_call_tool_result().unwrap();
        let CallToolResponse::Complete(result) = response else {
            panic!("expected a complete result");
        };
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_none());
        let text = result.content[0].as_text().unwrap().text.clone();
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["error"]["code"], "ENTITY_NOT_FOUND");
        assert_eq!(body["error"]["http_status"], 404);
        assert!(
            body["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("search_entities")
        );
    }

    #[test]
    fn unknown_codes_have_no_hint() {
        let error = ToolError::new("SOMETHING_NEW", "message");
        assert!(error.hint.is_none());
        assert!(error.to_json()["error"].get("hint").is_none());
    }
}
