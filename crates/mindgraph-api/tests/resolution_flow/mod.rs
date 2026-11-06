//! The M9 entity resolution flow through the HTTP router, shared by an
//! offline test (memory GraphStore) and an opt-in test against HugeGraph 1.7.
//!
//! ```text
//! graph A: two similar customers (+ accounts, OWNS) and a different one
//!          scan → candidate → compare → approve (canonical chosen) → identity
//!          then: both originals, their properties and relationships unchanged
//! graph B: the same customers, untouched by everything done on graph A
//! ```
//!
//! Metadata and resolution decisions live in the memory stores; graph data
//! lives in whichever GraphStore the test passes. No model is involved.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // Shared by several test binaries.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use mindgraph_api::{AppState, router};
use mindgraph_core::{
    CreateGraphProject, EntityResolutionConfig, EntityResolutionService, GraphExplorerService,
    GraphProjectService, IngestionConfig, IngestionPorts, IngestionService,
    MemoryEntityResolutionStore, MemoryMetadataStore, MetadataStore, OntologyDefinition,
    OntologyService,
};
use mindgraph_domain::{
    Entity, EntityId, GraphId, Properties, PropertyKey, PropertyValue, Relationship,
    RelationshipId, TypeName,
};
use mindgraph_graph::{EntitySearch, GraphStore};
use mindgraph_ingestion::MemorySourceStore;
use serde_json::{Value, json};
use tower::ServiceExt;

pub struct TestApp {
    pub router: Router,
    pub graph_id: GraphId,
    pub other_graph: GraphId,
    pub ikhwan_a: EntityId,
    pub ikhwan_b: EntityId,
    pub alice: EntityId,
    pub foreign: EntityId,
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

fn entity(graph_id: GraphId, entity_type: &str, values: &[(&str, &str)]) -> Entity {
    let mut properties = Properties::new();
    for (key, value) in values {
        properties
            .insert(PropertyKey::new(*key).unwrap(), PropertyValue::from(*value))
            .unwrap();
    }
    Entity::new(
        EntityId::new(),
        graph_id,
        TypeName::new(entity_type).unwrap(),
        properties,
    )
}

fn owns(graph_id: GraphId, customer: &Entity, account: &Entity) -> Relationship {
    let mut properties = Properties::new();
    properties
        .insert(PropertyKey::new("since").unwrap(), "2024")
        .unwrap();
    Relationship::new(
        RelationshipId::new(),
        graph_id,
        TypeName::new("OWNS").unwrap(),
        customer.id(),
        account.id(),
        properties,
    )
}

fn ontology() -> OntologyDefinition {
    serde_json::from_value(json!({
        "entity_types": [
            {"name": "Customer", "properties": [
                {"name": "customer_id", "type": "string", "required": true},
                {"name": "name", "type": "string"},
                {"name": "email", "type": "string"},
                {"name": "phone", "type": "string"},
                {"name": "age", "type": "integer"}]},
            {"name": "Account", "properties": [
                {"name": "account_id", "type": "string", "required": true}]}
        ],
        "relationship_types": [
            {"name": "OWNS", "source_entity_type": "Customer", "target_entity_type": "Account",
             "properties": [{"name": "since", "type": "string"}]}
        ]
    }))
    .unwrap()
}

/// Two graphs with synthetic customers written through `store`.
pub async fn app(store: Arc<dyn GraphStore>) -> TestApp {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let graphs = GraphProjectService::bootstrap(metadata.clone())
        .await
        .unwrap();
    let workspace_id = graphs.workspace_id();
    let ontologies = OntologyService::new(metadata.clone(), workspace_id);
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
            .create_version(graph.id(), &ontology())
            .await
            .unwrap();
        ids.push(graph.id());
    }
    let (graph_id, other_graph) = (ids[0], ids[1]);
    let ikhwan_a = entity(
        graph_id,
        "Customer",
        &[
            ("customer_id", "C101"),
            ("name", "Muhammad Ikhwan Nasution"),
            ("email", "ikhwan@example.com"),
            ("phone", "+62 812-1234-5678"),
        ],
    );
    let ikhwan_b = entity(
        graph_id,
        "Customer",
        &[
            ("customer_id", "K-2001"),
            ("name", "M. Ikhwan Nasution"),
            ("email", "IKHWAN@example.com"),
            ("phone", "0812 1234 5678"),
        ],
    );
    let alice = entity(
        graph_id,
        "Customer",
        &[("customer_id", "C001"), ("name", "Alice Tan")],
    );
    let account_a = entity(graph_id, "Account", &[("account_id", "A-101")]);
    let account_b = entity(graph_id, "Account", &[("account_id", "A-201")]);
    let foreign = entity(
        other_graph,
        "Customer",
        &[
            ("customer_id", "C101"),
            ("name", "Muhammad Ikhwan Nasution"),
        ],
    );
    let foreign_twin = entity(
        other_graph,
        "Customer",
        &[("customer_id", "K-2001"), ("name", "M. Ikhwan Nasution")],
    );
    store
        .create_entities(vec![
            ikhwan_a.clone(),
            ikhwan_b.clone(),
            alice.clone(),
            account_a.clone(),
            account_b.clone(),
        ])
        .await
        .unwrap();
    store
        .create_entities(vec![foreign.clone(), foreign_twin])
        .await
        .unwrap();
    store
        .create_relationships(vec![
            owns(graph_id, &ikhwan_a, &account_a),
            owns(graph_id, &ikhwan_b, &account_b),
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
    let explorer = GraphExplorerService::new(metadata.clone(), store, workspace_id);
    let resolution = EntityResolutionService::new(
        explorer.clone(),
        Arc::new(MemoryEntityResolutionStore::new()),
        EntityResolutionConfig::default(),
    );
    let state = AppState::new(
        graphs,
        OntologyService::new(metadata, workspace_id),
        ingestion,
    )
    .with_graph_explorer(explorer)
    .with_entity_resolution(resolution);
    TestApp {
        router: router(state),
        graph_id,
        other_graph,
        ikhwan_a: ikhwan_a.id(),
        ikhwan_b: ikhwan_b.id(),
        alice: alice.id(),
        foreign: foreign.id(),
    }
}

pub async fn send(app: &TestApp, request: Request<Body>) -> Reply {
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

pub async fn get(app: &TestApp, uri: &str) -> Reply {
    let request = Request::get(uri)
        .header("x-request-id", "client-request-1")
        .body(Body::empty())
        .unwrap();
    send(app, request).await
}

pub async fn post(app: &TestApp, uri: &str, body: &Value) -> Reply {
    let request = Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", "client-request-1")
        .body(Body::from(body.to_string()))
        .unwrap();
    send(app, request).await
}

pub fn assert_error(reply: &Reply, status: StatusCode, code: &str) {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert_eq!(reply.body["error"]["code"], code, "{}", reply.body);
    assert_eq!(reply.headers["x-request-id"], "client-request-1");
    assert_eq!(reply.body["error"]["request_id"], "client-request-1");
}

pub fn scan_body() -> Value {
    json!({
        "entity_type": "Customer",
        "max_entities": 400,
        "rules": [
            {"property": "customer_id", "kind": "exact_identifier"},
            {"property": "email", "kind": "normalized_email"},
            {"property": "phone", "kind": "normalized_phone", "country": "ID"},
            {"property": "name", "kind": "fuzzy_name", "threshold": 0.9}
        ]
    })
}

/// Every entity and relationship of a graph, read straight from the store.
pub async fn snapshot(
    store: &dyn GraphStore,
    graph_id: GraphId,
) -> (Vec<Entity>, Vec<Relationship>) {
    let entities = store
        .search_entities(graph_id, &EntitySearch::new().with_limit(1_000).unwrap())
        .await
        .unwrap();
    let ids: Vec<EntityId> = entities.iter().map(Entity::id).collect();
    let relationships = store
        .get_subgraph(graph_id, &ids)
        .await
        .unwrap()
        .relationships()
        .to_vec();
    (entities, relationships)
}

/// The definition of done of M9 on one GraphStore. Returns both graph ids so
/// a live test can clear them.
pub async fn run(app: &TestApp, store: &dyn GraphStore) -> [GraphId; 2] {
    let graph = app.graph_id;
    let before = snapshot(store, graph).await;
    let other_before = snapshot(store, app.other_graph).await;
    assert_eq!(before.0.len(), 5);
    assert_eq!(before.1.len(), 2);

    // Scan: one candidate, the two Ikhwan records.
    let scan = post(
        app,
        &format!("/api/v1/graphs/{graph}/resolution/scans"),
        &scan_body(),
    )
    .await;
    assert_eq!(scan.status, StatusCode::CREATED, "{}", scan.body);
    assert_eq!(scan.body["entities_examined"], 3);
    assert_eq!(scan.body["truncated"], false);
    assert_eq!(scan.body["exhaustive"], true);
    assert_eq!(scan.body["candidates_found"], 1);
    let scan_id = scan.body["scan_id"].as_str().unwrap().to_owned();
    assert!(
        scan.headers[header::LOCATION]
            .to_str()
            .unwrap()
            .ends_with(&scan_id)
    );

    let list = get(
        app,
        &format!("/api/v1/graphs/{graph}/resolution/candidates?status=pending"),
    )
    .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.body);
    assert_eq!(list.body["counts"]["pending"], 1);
    let candidate = &list.body["data"][0];
    let candidate_id = candidate["id"].as_str().unwrap().to_owned();
    let pair = [
        candidate["entity_a_id"].as_str().unwrap().to_owned(),
        candidate["entity_b_id"].as_str().unwrap().to_owned(),
    ];
    let mut expected = [app.ikhwan_a.to_string(), app.ikhwan_b.to_string()];
    expected.sort();
    assert_eq!(pair, expected);
    let outcome = |property: &str| {
        candidate["signals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["property"] == property)
            .unwrap()["outcome"]
            .clone()
    };
    assert_eq!(outcome("email"), "normalized");
    assert_eq!(outcome("phone"), "normalized");
    assert_eq!(outcome("name"), "fuzzy");
    assert_eq!(outcome("customer_id"), "no_match");

    // Compare: both original entities with their stored properties.
    let detail = get(
        app,
        &format!("/api/v1/graphs/{graph}/resolution/candidates/{candidate_id}"),
    )
    .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.body);
    let names: Vec<&str> = ["entity_a", "entity_b"]
        .iter()
        .map(|key| detail.body[key]["properties"]["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"M. Ikhwan Nasution") && names.contains(&"Muhammad Ikhwan Nasution"));

    // Approve with the reviewer's canonical choice.
    let decision = post(
        app,
        &format!("/api/v1/graphs/{graph}/resolution/candidates/{candidate_id}/decisions"),
        &json!({"decision": "same_entity", "canonical_entity_id": app.ikhwan_a.to_string()}),
    )
    .await;
    assert_eq!(decision.status, StatusCode::CREATED, "{}", decision.body);
    assert_eq!(decision.body["candidate"]["status"], "accepted");
    assert_eq!(
        decision.body["decision"]["canonical_entity_id"],
        app.ikhwan_a.to_string()
    );
    assert_eq!(decision.body["decision"]["scan_id"], scan_id);
    assert_eq!(decision.body["decision"]["actor"], "local-user");
    assert_eq!(decision.body["identity"]["physical_graph_merged"], false);

    // Inspect the identity group from the alias.
    let identity = get(
        app,
        &format!(
            "/api/v1/graphs/{graph}/resolution/entities/{}",
            app.ikhwan_b
        ),
    )
    .await;
    assert_eq!(identity.status, StatusCode::OK, "{}", identity.body);
    assert_eq!(
        identity.body["canonical_entity_id"],
        app.ikhwan_a.to_string()
    );
    assert_eq!(identity.body["is_canonical"], false);
    assert_eq!(identity.body["resolved"], true);
    assert_eq!(identity.body["resolution_type"], "manual");
    assert_eq!(identity.body["physical_graph_merged"], false);
    assert_eq!(
        identity.body["member_entity_ids"].as_array().unwrap().len(),
        2
    );

    // The graph is untouched: both originals, their properties and their
    // relationships; and the other graph is exactly as before.
    assert_eq!(snapshot(store, graph).await, before);
    assert_eq!(snapshot(store, app.other_graph).await, other_before);
    for id in [app.ikhwan_a, app.ikhwan_b] {
        let entity = get(app, &format!("/api/v1/graphs/{graph}/entities/{id}")).await;
        assert_eq!(entity.status, StatusCode::OK, "{}", entity.body);
        assert_eq!(
            entity.body["id"],
            id.to_string(),
            "no canonical substitution"
        );
    }
    let neighbours = get(
        app,
        &format!("/api/v1/graphs/{graph}/entities/{}/neighbors", app.ikhwan_b),
    )
    .await;
    assert_eq!(
        neighbours.body["relationships"].as_array().unwrap().len(),
        1
    );

    // The other graph has no candidates and knows nothing of the mapping.
    let other = get(
        app,
        &format!("/api/v1/graphs/{}/resolution/candidates", app.other_graph),
    )
    .await;
    assert_eq!(other.body["data"].as_array().unwrap().len(), 0);
    let foreign = get(
        app,
        &format!(
            "/api/v1/graphs/{}/resolution/entities/{}",
            app.other_graph, app.foreign
        ),
    )
    .await;
    assert_eq!(foreign.body["resolved"], false);
    [graph, app.other_graph]
}
