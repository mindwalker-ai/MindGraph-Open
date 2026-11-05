//! The MindGraph Core REST client: the server's only way to graph data.
//!
//! One `reqwest::Client` (one connection pool) serves every tool call. Each
//! request:
//!
//! - targets a path under `/api/v1/graphs/{configured graph id}` built here,
//!   never from a tool argument, so no call can reach another graph;
//! - carries a fresh `x-request-id` (UUIDv7) that Core logs and echoes;
//! - has a deadline (`MINDGRAPH_MCP_TIMEOUT_SECONDS`, or
//!   `MINDGRAPH_MCP_ASK_TIMEOUT_SECONDS` for GraphRAG);
//! - reads at most twice `MINDGRAPH_MCP_MAX_RESULT_BYTES` of body (Core's
//!   documents repeat `graph_id` on every element, which the tool results
//!   drop; the result itself is bounded again by the tools);
//! - never follows redirects and ignores proxy environment variables, so
//!   graph data cannot be sent anywhere but the configured loopback Core.
//!
//! Elements Core returns are checked to belong to the configured graph as a
//! second line of defence behind Core's own graph isolation.

use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderValue};
use reqwest::{Method, RequestBuilder, StatusCode, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::dto::{
    CoreAnswer, CoreEntity, CoreGraph, CoreHealth, CoreOntology, CorePath, CoreRelationship,
    CoreSearch, CoreSubgraph, Entity, Relationship,
};
use crate::errors::{ToolError, codes};

/// The header carrying the correlation id (ADR-013).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Longest time spent establishing a TCP connection to Core.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Neighbour direction, as Core names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Relationships starting at the entity.
    Outgoing,
    /// Relationships pointing to the entity.
    Incoming,
    /// Both.
    Both,
}

impl Direction {
    /// The query value Core expects.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
            Self::Both => "both",
        }
    }
}

/// Entities and relationships of the configured graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Elements {
    /// Entities, in Core's order.
    pub entities: Vec<Entity>,
    /// Relationships, in Core's order.
    pub relationships: Vec<Relationship>,
}

/// A shortest-path search result.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    /// Whether a path was found.
    pub found: bool,
    /// Its length in relationships.
    pub hops: usize,
    /// Entities and relationships in path order.
    pub elements: Elements,
}

/// Core's liveness answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// `ok` when Core serves requests.
    pub status: String,
    /// The service name (`mindgraph-core`).
    pub service: String,
    /// The Core version.
    pub version: String,
}

/// The REST client, bound to one Core and one graph.
#[derive(Debug, Clone)]
pub struct CoreClient {
    http: reqwest::Client,
    base: Url,
    graph_id: String,
    timeout: Duration,
    ask_timeout: Duration,
    max_body_bytes: usize,
}

impl CoreClient {
    /// Builds the client for `config`.
    ///
    /// # Errors
    ///
    /// The HTTP client cannot be initialised (no TLS or system resources).
    pub fn new(config: &Config) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT.min(config.timeout))
            .user_agent(concat!("mindgraph-mcp/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http,
            base: config.core_url.clone(),
            graph_id: config.graph_id.to_string(),
            timeout: config.timeout,
            ask_timeout: config.ask_timeout,
            max_body_bytes: config.max_result_bytes.saturating_mul(2),
        })
    }

    /// The configured graph id.
    #[must_use]
    pub fn graph_id(&self) -> &str {
        &self.graph_id
    }

    /// The Core base URL.
    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base
    }

    /// `GET /health`: whether Core is up.
    ///
    /// # Errors
    ///
    /// A [`ToolError`] when Core is unreachable or answers unexpectedly.
    pub async fn health(&self) -> Result<Health, ToolError> {
        let url = self.join("health");
        let health: CoreHealth = self
            .send("health", self.http.request(Method::GET, url), self.timeout)
            .await?;
        Ok(Health {
            status: health.status,
            service: health.service,
            version: health.version,
        })
    }

    /// `GET /api/v1/graphs/{id}`.
    pub(crate) async fn graph(&self) -> Result<CoreGraph, ToolError> {
        let request = self.http.request(Method::GET, self.graph_url(""));
        let graph: CoreGraph = self.send("get_graph", request, self.timeout).await?;
        if graph.id != self.graph_id {
            return Err(foreign_graph());
        }
        Ok(graph)
    }

    /// `GET /api/v1/graphs/{id}/ontology`.
    pub(crate) async fn ontology(&self) -> Result<CoreOntology, ToolError> {
        let request = self.http.request(Method::GET, self.graph_url("/ontology"));
        let ontology: CoreOntology = self.send("get_ontology", request, self.timeout).await?;
        if ontology.graph_id != self.graph_id {
            return Err(foreign_graph());
        }
        Ok(ontology)
    }

    /// `POST /api/v1/graphs/{id}/entities/search`.
    pub(crate) async fn search_entities(
        &self,
        entity_type: Option<&str>,
        property_filters: &serde_json::Map<String, Value>,
        limit: usize,
    ) -> Result<Vec<Entity>, ToolError> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            entity_type: Option<&'a str>,
            property_filters: &'a serde_json::Map<String, Value>,
            limit: usize,
        }
        let request = self
            .http
            .request(Method::POST, self.graph_url("/entities/search"))
            .json(&Body {
                entity_type,
                property_filters,
                limit,
            });
        let search: CoreSearch = self.send("search_entities", request, self.timeout).await?;
        self.entities(search.data)
    }

    /// `GET /api/v1/graphs/{id}/entities/{entity_id}`.
    pub(crate) async fn entity(&self, entity_id: Uuid) -> Result<Entity, ToolError> {
        let request = self.http.request(
            Method::GET,
            self.graph_url(&format!("/entities/{entity_id}")),
        );
        let entity: CoreEntity = self.send("get_entity", request, self.timeout).await?;
        self.own(entity.graph_id, entity.entity)
    }

    /// `GET /api/v1/graphs/{id}/entities/{entity_id}/neighbors`.
    pub(crate) async fn neighbors(
        &self,
        entity_id: Uuid,
        direction: Direction,
        relationship_type: Option<&str>,
        limit: usize,
    ) -> Result<Elements, ToolError> {
        let mut query = vec![
            ("direction", direction.as_str().to_owned()),
            ("limit", limit.to_string()),
        ];
        if let Some(relationship_type) = relationship_type {
            query.push(("relationship_type", relationship_type.to_owned()));
        }
        let request = self
            .http
            .request(
                Method::GET,
                self.graph_url(&format!("/entities/{entity_id}/neighbors")),
            )
            .query(&query);
        let subgraph: CoreSubgraph = self.send("get_neighbors", request, self.timeout).await?;
        self.elements(subgraph.entities, subgraph.relationships)
    }

    /// `POST /api/v1/graphs/{id}/paths`.
    pub(crate) async fn find_path(
        &self,
        source_id: Uuid,
        target_id: Uuid,
        direction: Direction,
        max_depth: usize,
    ) -> Result<Path, ToolError> {
        let body = serde_json::json!({
            "source_id": source_id.to_string(),
            "target_id": target_id.to_string(),
            "direction": direction.as_str(),
            "max_depth": max_depth,
        });
        let request = self
            .http
            .request(Method::POST, self.graph_url("/paths"))
            .json(&body);
        let path: CorePath = self.send("find_path", request, self.timeout).await?;
        Ok(Path {
            found: path.found,
            hops: path.hops,
            elements: self.elements(path.entities, path.relationships)?,
        })
    }

    /// `POST /api/v1/graphs/{id}/subgraph`.
    pub(crate) async fn subgraph(&self, entity_ids: &[Uuid]) -> Result<Elements, ToolError> {
        let ids: Vec<String> = entity_ids.iter().map(Uuid::to_string).collect();
        let request = self
            .http
            .request(Method::POST, self.graph_url("/subgraph"))
            .json(&serde_json::json!({ "entity_ids": ids }));
        let subgraph: CoreSubgraph = self.send("get_subgraph", request, self.timeout).await?;
        self.elements(subgraph.entities, subgraph.relationships)
    }

    /// `POST /api/v1/graphs/{id}/ask` (M7 GraphRAG). `entity_ids` empty asks
    /// the whole graph (bounded discovery).
    pub(crate) async fn ask(
        &self,
        question: &str,
        entity_ids: &[Uuid],
    ) -> Result<CoreAnswer, ToolError> {
        let scope = if entity_ids.is_empty() {
            serde_json::json!({ "mode": "bounded_graph" })
        } else {
            let ids: Vec<String> = entity_ids.iter().map(Uuid::to_string).collect();
            serde_json::json!({ "mode": "selected_entities", "entity_ids": ids })
        };
        let request = self
            .http
            .request(Method::POST, self.graph_url("/ask"))
            .json(&serde_json::json!({ "question": question, "scope": scope }));
        self.send("ask", request, self.ask_timeout).await
    }

    // --- plumbing ---------------------------------------------------------------------------

    fn join(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url
    }

    fn graph_url(&self, suffix: &str) -> Url {
        self.join(&format!("/api/v1/graphs/{}{suffix}", self.graph_id))
    }

    fn own<T>(&self, graph_id: String, element: T) -> Result<T, ToolError> {
        if graph_id == self.graph_id {
            Ok(element)
        } else {
            Err(foreign_graph())
        }
    }

    fn entities(&self, entities: Vec<CoreEntity>) -> Result<Vec<Entity>, ToolError> {
        entities
            .into_iter()
            .map(|entity| self.own(entity.graph_id, entity.entity))
            .collect()
    }

    fn elements(
        &self,
        entities: Vec<CoreEntity>,
        relationships: Vec<CoreRelationship>,
    ) -> Result<Elements, ToolError> {
        Ok(Elements {
            entities: self.entities(entities)?,
            relationships: relationships
                .into_iter()
                .map(|relationship| self.own(relationship.graph_id, relationship.relationship))
                .collect::<Result<_, _>>()?,
        })
    }

    /// Sends one request and decodes a successful JSON answer. Logs the
    /// operation, status, size and duration, never the bodies.
    async fn send<T: DeserializeOwned>(
        &self,
        operation: &'static str,
        request: RequestBuilder,
        timeout: Duration,
    ) -> Result<T, ToolError> {
        let request_id = Uuid::now_v7().to_string();
        let started = Instant::now();
        let result = self.exchange(request, timeout, &request_id).await;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match result {
            Ok((status, body)) => {
                info!(
                    operation,
                    request_id = %request_id,
                    status = status.as_u16(),
                    bytes = body.len(),
                    elapsed_ms,
                    "core request"
                );
                serde_json::from_slice(&body).map_err(|_| {
                    ToolError::new(
                        codes::INVALID_CORE_RESPONSE,
                        "MindGraph Core returned a response this version of mindgraph-mcp cannot read",
                    )
                    .with_request(Some(status.as_u16()), &request_id)
                })
            }
            Err(error) => {
                warn!(
                    operation,
                    request_id = %request_id,
                    code = %error.code,
                    status = error.http_status,
                    elapsed_ms,
                    "core request failed"
                );
                Err(error)
            }
        }
    }

    async fn exchange(
        &self,
        request: RequestBuilder,
        timeout: Duration,
        request_id: &str,
    ) -> Result<(StatusCode, Vec<u8>), ToolError> {
        let mut response = request
            .timeout(timeout)
            .header(ACCEPT, "application/json")
            .header(REQUEST_ID_HEADER, request_id)
            .send()
            .await
            .map_err(|error| self.transport_error(&error, request_id))?;
        let status = response.status();
        if status.is_redirection() {
            return Err(ToolError::new(
                codes::CORE_REDIRECT,
                format!(
                    "MindGraph Core answered with a redirect (HTTP {}), which mindgraph-mcp never follows",
                    status.as_u16()
                ),
            )
            .with_hint("Point MINDGRAPH_MCP_CORE_URL at MindGraph Core itself, not a proxy or the web app.")
            .with_request(Some(status.as_u16()), request_id));
        }
        let too_large = || {
            ToolError::new(
                codes::RESULT_TOO_LARGE,
                format!(
                    "MindGraph Core's answer exceeds {} bytes",
                    self.max_body_bytes
                ),
            )
            .with_request(Some(status.as_u16()), request_id)
        };
        if response
            .content_length()
            .is_some_and(|length| length > self.max_body_bytes as u64)
        {
            return Err(too_large());
        }
        let is_json = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value: &HeaderValue| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"));
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| self.transport_error(&error, request_id))?
        {
            if body.len() + chunk.len() > self.max_body_bytes {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        if status.is_success() {
            if !is_json {
                return Err(ToolError::new(
                    codes::INVALID_CORE_RESPONSE,
                    "MindGraph Core did not answer with JSON; is MINDGRAPH_MCP_CORE_URL pointing at MindGraph Core?",
                )
                .with_request(Some(status.as_u16()), request_id));
            }
            return Ok((status, body));
        }
        Err(core_error(status, &body, request_id))
    }

    fn transport_error(&self, error: &reqwest::Error, request_id: &str) -> ToolError {
        let error = if error.is_timeout() {
            ToolError::new(
                codes::CORE_TIMEOUT,
                "MindGraph Core did not answer within the deadline",
            )
        } else {
            ToolError::new(
                codes::CORE_UNAVAILABLE,
                format!(
                    "MindGraph Core is not reachable at {}",
                    self.base.as_str().trim_end_matches('/')
                ),
            )
        };
        error.with_request(None, request_id)
    }
}

fn foreign_graph() -> ToolError {
    ToolError::new(
        codes::INVALID_CORE_RESPONSE,
        "MindGraph Core returned data of another graph; the result was discarded",
    )
}

/// A Core error: its envelope (`{"error": {"code", "message"}}`) when it has
/// one, a generic error otherwise. Non-envelope bodies (an HTML page from a
/// proxy, a stack trace) are never passed on.
fn core_error(status: StatusCode, body: &[u8], request_id: &str) -> ToolError {
    #[derive(serde::Deserialize)]
    struct Envelope {
        error: Detail,
    }
    #[derive(serde::Deserialize)]
    struct Detail {
        code: String,
        message: String,
        #[serde(default)]
        request_id: Option<String>,
    }
    match serde_json::from_slice::<Envelope>(body) {
        Ok(Envelope { error }) => {
            let request_id = error.request_id.unwrap_or_else(|| request_id.to_owned());
            ToolError::new(error.code, error.message)
                .with_request(Some(status.as_u16()), &request_id)
        }
        Err(_) => ToolError::new(
            codes::CORE_ERROR,
            format!(
                "MindGraph Core answered HTTP {} without an error envelope; is MINDGRAPH_MCP_CORE_URL pointing at MindGraph Core?",
                status.as_u16()
            ),
        )
        .with_request(Some(status.as_u16()), request_id),
    }
}

#[cfg(test)]
mod tests;
