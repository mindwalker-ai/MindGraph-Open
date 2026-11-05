//! Graph Explorer HTTP endpoints over the in-memory stores. Offline: no
//! PostgreSQL, no HugeGraph.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use mindgraph_api::{AppState, router};
use mindgraph_core::{
    CreateGraphProject, GraphExplorerService, GraphProjectService, IngestionConfig, IngestionPorts,
    IngestionService, MemoryMetadataStore, MetadataStore, OntologyDefinition, OntologyService,
};
use mindgraph_domain::{
    Entity, EntityId, GraphId, Properties, PropertyKey, PropertyValue, Relationship,
    RelationshipId, TypeName,
};
use mindgraph_graph::{GraphStore, MemoryGraphStore};
use mindgraph_ingestion::MemorySourceStore;
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestApp {
    router: Router,
    graph: String,
    other_graph: String,
    customer_a: String,
    customer_b: String,
    account: String,
    device: String,
    foreign: String,
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

fn entity(graph_id: GraphId, entity_type: &str, values: &[(&str, PropertyValue)]) -> Entity {
    let mut properties = Properties::new();
    for (key, value) in values {
        properties
            .insert(PropertyKey::new(*key).unwrap(), value.clone())
            .unwrap();
    }
    Entity::new(
        EntityId::new(),
        graph_id,
        TypeName::new(entity_type).unwrap(),
        properties,
    )
}

fn relationship(graph_id: GraphId, kind: &str, source: &Entity, target: &Entity) -> Relationship {
    Relationship::new(
        RelationshipId::new(),
        graph_id,
        TypeName::new(kind).unwrap(),
        source.id(),
        target.id(),
        Properties::new(),
    )
}

async fn app() -> TestApp {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let store = Arc::new(MemoryGraphStore::new());
    let graphs = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let workspace_id = graphs.workspace_id();
    let ontologies = OntologyService::new(metadata.clone(), workspace_id);
    let definition: OntologyDefinition = serde_json::from_value(json!({
        "entity_types": [
            {"name": "Customer", "properties": [
                {"name": "customer_id", "type": "string", "required": true},
                {"name": "name", "type": "string"},
                {"name": "score", "type": "integer"},
                {"name": "joined_at", "type": "datetime"},
                {"name": "tags", "type": "string_array"}]},
            {"name": "Account", "properties": [
                {"name": "account_id", "type": "string", "required": true},
                {"name": "balance", "type": "float"}]},
            {"name": "Device", "properties": [
                {"name": "device_id", "type": "string", "required": true}]}
        ],
        "relationship_types": [
            {"name": "OWNS", "source_entity_type": "Customer", "target_entity_type": "Account"},
            {"name": "USES", "source_entity_type": "Customer", "target_entity_type": "Device"}
        ]
    }))
    .unwrap();
    let mut ids = Vec::new();
    for name in ["Fraud", "Other"] {
        let graph = graphs
            .create_graph(CreateGraphProject {
                name: name.into(),
                description: None,
            })
            .await
            .unwrap();
        ontologies
            .create_version(graph.id(), &definition)
            .await
            .unwrap();
        ids.push(graph.id());
    }
    let (graph_id, other) = (ids[0], ids[1]);
    let string = |value: &str| PropertyValue::String(value.into());
    let a = entity(
        graph_id,
        "Customer",
        &[
            ("customer_id", string("C001")),
            ("name", string("<b>Alice</b>")),
            ("score", PropertyValue::Integer(9_007_199_254_740_993)),
            (
                "joined_at",
                PropertyValue::DateTime("2024-02-11T09:30:00Z".parse().unwrap()),
            ),
            ("tags", PropertyValue::StringArray(vec!["vip".into()])),
        ],
    );
    let b = entity(graph_id, "Customer", &[("customer_id", string("C002"))]);
    let account = entity(
        graph_id,
        "Account",
        &[
            ("account_id", string("A1")),
            ("balance", PropertyValue::Float(10.5)),
        ],
    );
    let device = entity(graph_id, "Device", &[("device_id", string("DEVICE-921"))]);
    let foreign = entity(other, "Customer", &[("customer_id", string("C001"))]);
    store
        .create_entities(vec![a.clone(), b.clone(), account.clone(), device.clone()])
        .await
        .unwrap();
    store.create_entity(foreign.clone()).await.unwrap();
    store
        .create_relationships(vec![
            relationship(graph_id, "OWNS", &a, &account),
            relationship(graph_id, "USES", &a, &device),
            relationship(graph_id, "USES", &b, &device),
        ])
        .await
        .unwrap();
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata: metadata.clone(),
            graph: store.clone(),
            sources: Arc::new(MemorySourceStore::new()),
        },
        workspace_id,
        IngestionConfig::default(),
    )
    .unwrap();
    let state = AppState::new(
        graphs,
        OntologyService::new(metadata.clone(), workspace_id),
        ingestion,
    )
    .with_graph_explorer(GraphExplorerService::new(metadata, store, workspace_id));
    TestApp {
        router: router(state),
        graph: graph_id.to_string(),
        other_graph: other.to_string(),
        customer_a: a.id().to_string(),
        customer_b: b.id().to_string(),
        account: account.id().to_string(),
        device: device.id().to_string(),
        foreign: foreign.id().to_string(),
    }
}

async fn send(app: &TestApp, request: Request<Body>) -> Reply {
    let response = app.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

async fn get(app: &TestApp, uri: &str) -> Reply {
    let request = Request::get(uri)
        .header("x-request-id", "client-request-1")
        .body(Body::empty())
        .unwrap();
    send(app, request).await
}

async fn post(app: &TestApp, uri: &str, body: &Value) -> Reply {
    let request = Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", "client-request-1")
        .body(Body::from(body.to_string()))
        .unwrap();
    send(app, request).await
}

fn assert_error(reply: &Reply, status: StatusCode, code: &str) {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert_eq!(reply.body["error"]["code"], code, "{}", reply.body);
    assert_eq!(reply.headers["x-request-id"], "client-request-1");
    assert_eq!(reply.body["error"]["request_id"], "client-request-1");
}

fn ids(list: &Value) -> Vec<String> {
    let mut ids: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[tokio::test]
async fn entities_are_found_and_rendered_with_their_types() {
    let app = app().await;
    let uri = format!("/api/v1/graphs/{}/entities/search", app.graph);
    let reply = post(
        &app,
        &uri,
        &json!({"entity_type": "Customer", "property_filters": {"customer_id": "C001"}, "limit": 50}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.headers["x-request-id"], "client-request-1");
    assert_eq!(reply.body["count"], 1);
    assert_eq!(reply.body["limit"], 50);
    let customer = &reply.body["data"][0];
    assert_eq!(customer["id"], app.customer_a.as_str());
    assert_eq!(customer["graph_id"], app.graph.as_str());
    assert_eq!(customer["entity_type"], "Customer");
    assert_eq!(
        customer["properties"]["name"], "<b>Alice</b>",
        "values are data, never HTML"
    );
    assert_eq!(
        customer["properties"]["score"], "9007199254740993",
        "no precision loss"
    );
    assert_eq!(customer["property_types"]["score"], "integer");
    assert_eq!(customer["properties"]["joined_at"], "2024-02-11T09:30:00Z");
    assert_eq!(customer["property_types"]["joined_at"], "datetime");
    assert_eq!(customer["properties"]["tags"], json!(["vip"]));
    assert_eq!(customer["property_types"]["tags"], "string_array");

    let by_type = post(&app, &uri, &json!({"entity_type": "Customer"})).await;
    assert_eq!(by_type.body["count"], 2);
    let none = post(
        &app,
        &uri,
        &json!({"property_filters": {"customer_id": "C404"}}),
    )
    .await;
    assert_eq!(none.status, StatusCode::OK);
    assert_eq!(none.body["data"], json!([]));

    for bad in [
        json!({"limit": 0}),
        json!({"limit": 1001}),
        json!({"entity_type": "Planet"}),
        json!({"property_filters": {"unknown": "x"}}),
        json!({"entity_type": "Customer", "property_filters": {"score": "high"}}),
    ] {
        assert_error(
            &post(&app, &uri, &bad).await,
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
        );
    }
    assert_error(
        &post(&app, &uri, &json!({"query": "alice"})).await,
        StatusCode::BAD_REQUEST,
        "INVALID_JSON",
    );
}

#[tokio::test]
async fn entities_are_read_by_id_within_their_graph() {
    let app = app().await;
    let reply = get(
        &app,
        &format!("/api/v1/graphs/{}/entities/{}", app.graph, app.account),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["properties"]["balance"], 10.5);
    assert_eq!(reply.body["property_types"]["balance"], "float");

    for (graph, entity) in [
        (&app.graph, EntityId::new().to_string()),
        (&app.graph, app.foreign.clone()),
        (&app.other_graph, app.customer_a.clone()),
    ] {
        assert_error(
            &get(&app, &format!("/api/v1/graphs/{graph}/entities/{entity}")).await,
            StatusCode::NOT_FOUND,
            "ENTITY_NOT_FOUND",
        );
    }
    assert_error(
        &get(
            &app,
            &format!("/api/v1/graphs/{}/entities/not-an-id", app.graph),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_ENTITY_ID",
    );
    assert_error(
        &get(
            &app,
            &format!("/api/v1/graphs/{}/entities/{}", GraphId::new(), app.account),
        )
        .await,
        StatusCode::NOT_FOUND,
        "GRAPH_NOT_FOUND",
    );
    assert_error(
        &get(
            &app,
            &format!("/api/v1/graphs/nope/entities/{}", app.account),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_GRAPH_ID",
    );
}

#[tokio::test]
async fn neighbours_are_a_complete_subgraph() {
    let app = app().await;
    let base = format!(
        "/api/v1/graphs/{}/entities/{}/neighbors",
        app.graph, app.customer_a
    );
    let reply = get(&app, &base).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        ids(&reply.body["entities"]),
        sorted(vec![
            app.customer_a.clone(),
            app.account.clone(),
            app.device.clone()
        ])
    );
    let relationships = reply.body["relationships"].as_array().unwrap();
    assert_eq!(relationships.len(), 2);
    let owns = relationships
        .iter()
        .find(|r| r["relationship_type"] == "OWNS")
        .unwrap();
    assert_eq!(owns["source_id"], app.customer_a.as_str());
    assert_eq!(owns["target_id"], app.account.as_str());

    let device = format!(
        "/api/v1/graphs/{}/entities/{}/neighbors?direction=incoming&relationship_type=USES&limit=1",
        app.graph, app.device
    );
    let limited = get(&app, &device).await;
    assert_eq!(limited.status, StatusCode::OK, "{}", limited.body);
    assert_eq!(limited.body["relationships"].as_array().unwrap().len(), 1);

    for query in [
        "?limit=0",
        "?limit=1001",
        "?direction=up",
        "?relationship_type=a%20b",
        "?depth=2",
    ] {
        assert_error(
            &get(&app, &format!("{base}{query}")).await,
            StatusCode::BAD_REQUEST,
            "INVALID_QUERY",
        );
    }
    assert_error(
        &get(
            &app,
            &format!(
                "/api/v1/graphs/{}/entities/{}/neighbors",
                app.graph, app.foreign
            ),
        )
        .await,
        StatusCode::NOT_FOUND,
        "ENTITY_NOT_FOUND",
    );
}

#[tokio::test]
async fn shortest_paths_or_found_false() {
    let app = app().await;
    let uri = format!("/api/v1/graphs/{}/paths", app.graph);
    let reply = post(
        &app,
        &uri,
        &json!({"source_id": app.customer_a, "target_id": app.customer_b, "direction": "both", "max_depth": 4}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["found"], true);
    assert_eq!(reply.body["hops"], 2);
    let order: Vec<&str> = reply.body["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        order,
        [
            app.customer_a.as_str(),
            app.device.as_str(),
            app.customer_b.as_str()
        ]
    );
    assert_eq!(reply.body["relationships"].as_array().unwrap().len(), 2);

    let none = post(
        &app,
        &uri,
        &json!({"source_id": app.customer_a, "target_id": app.customer_b, "max_depth": 1}),
    )
    .await;
    assert_eq!(none.status, StatusCode::OK);
    assert_eq!(
        none.body,
        json!({"found": false, "hops": 0, "entities": [], "relationships": []})
    );

    for depth in [0, 7] {
        assert_error(
            &post(
                &app,
                &uri,
                &json!({"source_id": app.customer_a, "target_id": app.customer_b, "max_depth": depth}),
            )
            .await,
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
        );
    }
    assert_error(
        &post(
            &app,
            &uri,
            &json!({"source_id": "x", "target_id": app.customer_b}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_ENTITY_ID",
    );
    assert_error(
        &post(
            &app,
            &uri,
            &json!({"source_id": app.customer_a, "target_id": app.foreign}),
        )
        .await,
        StatusCode::NOT_FOUND,
        "ENTITY_NOT_FOUND",
    );
}

#[tokio::test]
async fn subgraphs_are_bounded() {
    let app = app().await;
    let uri = format!("/api/v1/graphs/{}/subgraph", app.graph);
    let reply = post(
        &app,
        &uri,
        &json!({"entity_ids": [app.customer_a, app.account, app.foreign]}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        ids(&reply.body["entities"]),
        sorted(vec![app.customer_a.clone(), app.account.clone()]),
        "other graphs are invisible"
    );
    assert_eq!(reply.body["relationships"][0]["relationship_type"], "OWNS");

    let empty = post(&app, &uri, &json!({"entity_ids": []})).await;
    assert_eq!(empty.body, json!({"entities": [], "relationships": []}));

    let too_many: Vec<String> = (0..1001).map(|_| EntityId::new().to_string()).collect();
    assert_error(
        &post(&app, &uri, &json!({"entity_ids": too_many})).await,
        StatusCode::BAD_REQUEST,
        "INVALID_REQUEST",
    );
    assert_error(
        &post(&app, &uri, &json!({"entity_ids": ["nope"]})).await,
        StatusCode::BAD_REQUEST,
        "INVALID_ENTITY_ID",
    );
}
