//! Shared test support: an MCP session with the real `mindgraph-mcp`
//! binary, an in-process MindGraph Core, and the fraud example loaded
//! through Core's public REST API.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // Test helpers; not every suite uses all of them.

use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use mindgraph_api::{AppState, router};
use mindgraph_core::{
    AiError, AiMetadata, AiStatus, GraphExplorerService, GraphProjectService, GraphRagAi,
    GraphRagConfig, GraphRagService, IngestionConfig, IngestionPorts, IngestionService,
    Interpretation, InterpretationRequest, MemoryMetadataStore, MetadataStore, OntologyService,
    Synthesis, SynthesisRequest, SynthesizedAnswer,
};
use mindgraph_domain::{Entity, EntityId, GraphId, Relationship};
use mindgraph_graph::{
    EntitySearch, GraphPath, GraphStore, GraphStoreError, NeighborQuery, PathQuery, Subgraph,
};
use mindgraph_ingestion::MemorySourceStore;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::process::Command;

pub const BIN: &str = env!("CARGO_BIN_EXE_mindgraph-mcp");

/// A fresh UUIDv7, e.g. a graph id that does not exist.
pub fn uuid_v7() -> String {
    uuid::Uuid::now_v7().to_string()
}

// --- MCP session ---------------------------------------------------------------------------

pub type Session = RunningService<RoleClient, ()>;

/// The `mindgraph-mcp` command for `core_url` and `graph_id`, with every
/// inherited `MINDGRAPH_MCP_*` variable removed.
pub fn command(core_url: &str, graph_id: &str, env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(BIN);
    for (key, _) in std::env::vars() {
        if key.starts_with("MINDGRAPH_MCP_") {
            command.env_remove(key);
        }
    }
    command
        .env("MINDGRAPH_MCP_CORE_URL", core_url)
        .env("MINDGRAPH_MCP_GRAPH_ID", graph_id)
        .env("RUST_LOG", "mindgraph_mcp=debug");
    for (key, value) in env {
        command.env(key, value);
    }
    command
}

/// Launches the real binary and connects the rmcp client over its stdio.
pub async fn connect(core_url: &str, graph_id: &str, env: &[(&str, &str)]) -> Session {
    let (transport, _stderr) = TokioChildProcess::builder(command(core_url, graph_id, env))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    ().serve(transport).await.unwrap()
}

/// Calls a tool; protocol errors panic, tool errors are returned.
pub async fn call(session: &Session, tool: &str, arguments: Value) -> CallToolResult {
    let Value::Object(arguments) = arguments else {
        panic!("tool arguments must be an object");
    };
    session
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments))
        .await
        .unwrap()
}

/// The structured content of a successful call, checked against its text.
pub fn ok(result: &CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result
        .structured_content
        .clone()
        .expect("structured content");
    let text = result.content[0]
        .as_text()
        .expect("a text block")
        .text
        .clone();
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        structured,
        "the text block is the same JSON"
    );
    structured
}

/// The error document of a failed call. Argument errors the SDK reports
/// before the tool runs are plain text; they get the code
/// `SDK_ARGUMENT_ERROR` here.
pub fn error(result: &CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(true), "{result:?}");
    assert!(result.structured_content.is_none(), "{result:?}");
    let text = &result.content[0].as_text().expect("a text block").text;
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|body| body.get("error").cloned())
        .unwrap_or_else(|| json!({ "code": "SDK_ARGUMENT_ERROR", "message": text }))
}

/// The error code of a failed call.
pub fn code(result: &CallToolResult) -> String {
    error(result)["code"].as_str().unwrap().to_owned()
}

// --- in-process MindGraph Core --------------------------------------------------------------

/// Counts every write that reaches the GraphStore.
pub struct WriteSpy {
    inner: Arc<dyn GraphStore>,
    writes: AtomicUsize,
}

impl WriteSpy {
    pub fn new(inner: Arc<dyn GraphStore>) -> Self {
        Self {
            inner,
            writes: AtomicUsize::new(0),
        }
    }

    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }

    fn write(&self) {
        self.writes.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl GraphStore for WriteSpy {
    async fn create_entity(&self, entity: Entity) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.create_entity(entity).await
    }
    async fn create_entities(&self, entities: Vec<Entity>) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.create_entities(entities).await
    }
    async fn create_relationship(&self, relationship: Relationship) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.create_relationship(relationship).await
    }
    async fn create_relationships(
        &self,
        relationships: Vec<Relationship>,
    ) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.create_relationships(relationships).await
    }
    async fn get_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<Option<Entity>, GraphStoreError> {
        self.inner.get_entity(graph_id, entity_id).await
    }
    async fn search_entities(
        &self,
        graph_id: GraphId,
        search: &EntitySearch,
    ) -> Result<Vec<Entity>, GraphStoreError> {
        self.inner.search_entities(graph_id, search).await
    }
    async fn get_neighbors(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
        query: &NeighborQuery,
    ) -> Result<Subgraph, GraphStoreError> {
        self.inner.get_neighbors(graph_id, entity_id, query).await
    }
    async fn find_path(
        &self,
        graph_id: GraphId,
        source: EntityId,
        target: EntityId,
        query: &PathQuery,
    ) -> Result<Option<GraphPath>, GraphStoreError> {
        self.inner.find_path(graph_id, source, target, query).await
    }
    async fn get_subgraph(
        &self,
        graph_id: GraphId,
        entity_ids: &[EntityId],
    ) -> Result<Subgraph, GraphStoreError> {
        self.inner.get_subgraph(graph_id, entity_ids).await
    }
    async fn delete_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<bool, GraphStoreError> {
        self.write();
        self.inner.delete_entity(graph_id, entity_id).await
    }
    async fn clear_graph(&self, graph_id: GraphId) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.clear_graph(graph_id).await
    }
}

/// A deterministic stand-in for the model behind `ask_graph`: it asks Core
/// for the customers sharing DEVICE-921 and cites every retrieved entity.
struct FakeModel;

fn fake_metadata(prompt_version: &str) -> AiMetadata {
    AiMetadata {
        prompt_version: prompt_version.into(),
        provider: "fake".into(),
        model: "deterministic-test".into(),
    }
}

#[async_trait]
impl GraphRagAi for FakeModel {
    async fn status(&self) -> AiStatus {
        AiStatus::Ready {
            provider: "fake".into(),
            model: "deterministic-test".into(),
        }
    }

    async fn interpret(&self, _request: &InterpretationRequest) -> Result<Interpretation, AiError> {
        let intent = json!({
            "operation": "shared_neighbor",
            "source": {"entity_type": "Device", "match": {"device_id": "DEVICE-921"}},
            "entity_type": "Customer", "shared_entity_type": "Device", "relationship_type": "USES"
        });
        Ok(Interpretation {
            intent: serde_json::from_value(intent).unwrap(),
            metadata: fake_metadata("graphrag_interpret_v1"),
        })
    }

    async fn synthesize(&self, request: &SynthesisRequest) -> Result<Synthesis, AiError> {
        let refs: Vec<String> = request.context["entities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entity| entity["ref"].as_str().unwrap().to_owned())
            .collect();
        Ok(Synthesis {
            answer: SynthesizedAnswer {
                answer: format!("These customers share the device [{}].", refs.join(", ")),
                evidence_refs: refs,
                insufficient_evidence: false,
                warnings: vec![],
            },
            metadata: fake_metadata("graphrag_synthesize_v1"),
        })
    }
}

/// A running MindGraph Core (the real router) on a loopback port.
pub struct Core {
    pub url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Core {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Serves Core's router over `graph` with in-memory metadata and sources,
/// and GraphRAG backed by [`FakeModel`].
pub async fn spawn_core(graph: Arc<dyn GraphStore>) -> Core {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let projects = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let workspace_id = projects.workspace_id();
    let explorer = GraphExplorerService::new(metadata.clone(), graph.clone(), workspace_id);
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata: metadata.clone(),
            graph,
            sources: Arc::new(MemorySourceStore::new()),
        },
        workspace_id,
        IngestionConfig::default(),
    )
    .unwrap();
    let app = router(
        AppState::new(
            projects,
            OntologyService::new(metadata, workspace_id),
            ingestion,
        )
        .with_graph_explorer(explorer.clone())
        .with_graphrag(GraphRagService::new(
            explorer,
            Arc::new(FakeModel),
            GraphRagConfig::default(),
        )),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Core { url, task }
}

// --- the fraud example through the REST API ------------------------------------------------

const ONTOLOGY: &str = include_str!("../../../../examples/fraud/ontology.json");
const FILES: [(&str, &str, &str); 3] = [
    (
        "customers.csv",
        include_str!("../../../../examples/fraud/customers.csv"),
        include_str!("../../../../examples/fraud/customers.mapping.json"),
    ),
    (
        "accounts.csv",
        include_str!("../../../../examples/fraud/accounts.csv"),
        include_str!("../../../../examples/fraud/accounts.mapping.json"),
    ),
    (
        "devices.csv",
        include_str!("../../../../examples/fraud/devices.csv"),
        include_str!("../../../../examples/fraud/devices.mapping.json"),
    ),
];
const BOUNDARY: &str = "mindgraph-mcp-test-boundary";

/// The two graphs of the example: A holds customers, accounts and devices;
/// B only customers (the same business keys, different entities).
pub struct Fraud {
    pub graph_a: String,
    pub graph_b: String,
}

async fn expect_json(response: reqwest::Response, status: u16) -> Value {
    let actual = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    assert_eq!(actual, status, "{body}");
    body
}

async fn wait_for_job(http: &reqwest::Client, core: &str, job_id: &str) {
    for _ in 0..1_200 {
        let job = expect_json(
            http.get(format!("{core}/api/v1/jobs/{job_id}"))
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        match job["status"].as_str() {
            Some("completed") => return,
            Some("failed") => panic!("job {job_id} failed: {job}"),
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    panic!("job {job_id} did not finish");
}

async fn create_graph(http: &reqwest::Client, core: &str, name: &str) -> String {
    let graph = expect_json(
        http.post(format!("{core}/api/v1/graphs"))
            .json(&json!({ "name": name, "description": "Synthetic fraud example" }))
            .send()
            .await
            .unwrap(),
        201,
    )
    .await;
    let id = graph["id"].as_str().unwrap().to_owned();
    expect_json(
        http.post(format!("{core}/api/v1/graphs/{id}/ontology"))
            .header("content-type", "application/json")
            .body(ONTOLOGY)
            .send()
            .await
            .unwrap(),
        201,
    )
    .await;
    id
}

async fn load_file(http: &reqwest::Client, core: &str, graph: &str, file: (&str, &str, &str)) {
    let (filename, content, mapping) = file;
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
         Content-Type: text/csv\r\n\r\n{content}\r\n--{BOUNDARY}--\r\n"
    );
    let source = expect_json(
        http.post(format!("{core}/api/v1/graphs/{graph}/data-sources"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .body(body)
            .send()
            .await
            .unwrap(),
        201,
    )
    .await;
    let base = format!(
        "{core}/api/v1/graphs/{graph}/data-sources/{}",
        source["id"].as_str().unwrap()
    );
    let analysis = expect_json(
        http.post(format!("{base}/analyze"))
            .json(&json!({}))
            .send()
            .await
            .unwrap(),
        202,
    )
    .await;
    wait_for_job(http, core, analysis["job_id"].as_str().unwrap()).await;
    let job = expect_json(
        http.post(format!("{base}/ingestions"))
            .header("content-type", "application/json")
            .header("idempotency-key", format!("{graph}-{filename}"))
            .body(mapping.to_owned())
            .send()
            .await
            .unwrap(),
        202,
    )
    .await;
    wait_for_job(http, core, job["id"].as_str().unwrap()).await;
}

/// Creates graph A (customers, accounts, devices) and graph B (customers
/// only) through Core's REST API, as a user would.
pub async fn load_fraud(core: &str, prefix: &str) -> Fraud {
    let http = reqwest::Client::new();
    let graph_a = create_graph(&http, core, &format!("{prefix} A")).await;
    let graph_b = create_graph(&http, core, &format!("{prefix} B")).await;
    for file in FILES {
        load_file(&http, core, &graph_a, file).await;
    }
    load_file(&http, core, &graph_b, FILES[0]).await;
    Fraud { graph_a, graph_b }
}

/// `GET` a Core path and return its JSON.
pub async fn core_get(core: &str, path: &str) -> Value {
    let response = reqwest::get(format!("{core}{path}")).await.unwrap();
    expect_json(response, 200).await
}

/// The id of the one entity of a type whose `key` equals `value`, read
/// directly from Core's REST API: the ground truth MCP results are checked
/// against.
pub async fn core_find(
    core: &str,
    graph: &str,
    entity_type: &str,
    key: &str,
    value: &str,
) -> String {
    let response = reqwest::Client::new()
        .post(format!("{core}/api/v1/graphs/{graph}/entities/search"))
        .json(&json!({"entity_type": entity_type, "property_filters": {key: value}}))
        .send()
        .await
        .unwrap();
    let body = expect_json(response, 200).await;
    assert_eq!(body["count"], 1, "{body}");
    body["data"][0]["id"].as_str().unwrap().to_owned()
}
