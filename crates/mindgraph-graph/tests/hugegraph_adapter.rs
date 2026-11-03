//! Offline tests of the HugeGraph adapter against a local mock HTTP server.
//!
//! They check the requests MindGraph sends to HugeGraph 1.7.0 and how HTTP
//! outcomes map to `GraphStoreError`, without Docker or network access.

#![allow(clippy::unwrap_used)]

use std::time::Duration;

use mindgraph_domain::{Entity, EntityId, GraphId, Properties, PropertyKey, TypeName};
use mindgraph_graph::{GraphStore, GraphStoreError, HugeGraphConfig, HugeGraphStore};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "/graphspaces/DEFAULT/graphs/hugegraph";

fn store_for(server: &MockServer) -> HugeGraphStore {
    let config = HugeGraphConfig::new(&server.uri())
        .unwrap()
        .with_request_timeout(Duration::from_secs(2))
        .unwrap();
    HugeGraphStore::new(config).unwrap()
}

fn customer(graph_id: GraphId) -> Entity {
    let mut properties = Properties::new();
    properties
        .insert(PropertyKey::new("name").unwrap(), "Customer A")
        .unwrap();
    Entity::new(
        EntityId::new(),
        graph_id,
        TypeName::new("Customer").unwrap(),
        properties,
    )
}

/// The vertex JSON HugeGraph would return for `entity`.
fn vertex_json(entity: &Entity) -> Value {
    json!({
        "id": format!("{}:{}", entity.graph_id(), entity.id()),
        "label": "mindgraph_entity",
        "type": "vertex",
        "properties": {
            "mg_graph_id": entity.graph_id().to_string(),
            "mg_type": entity.entity_type().to_string(),
            "mg_properties": r#"{"version":1,"properties":{"name":{"type":"string","value":"Customer A"}}}"#,
        }
    })
}

async fn mount_vertex_lookup(server: &MockServer, vertices: Value) {
    Mock::given(method("GET"))
        .and(path(format!("{ROOT}/traversers/vertices")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "vertices": vertices })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn ensure_schema_creates_every_element_idempotently() {
    let server = MockServer::start().await;
    for (endpoint, status, count) in [
        ("propertykeys", 202, 4),
        ("vertexlabels", 201, 1),
        ("edgelabels", 201, 1),
        ("indexlabels", 202, 2),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("{ROOT}/schema/{endpoint}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({})))
            .expect(count)
            .mount(&server)
            .await;
    }
    store_for(&server).ensure_schema().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    for request in &requests {
        let body: Value = request.body_json().unwrap();
        assert_eq!(body["check_exist"], json!(false), "{}", request.url);
    }
    let edge_label = requests
        .iter()
        .find(|r| r.url.path().ends_with("/edgelabels"))
        .unwrap()
        .body_json::<Value>()
        .unwrap();
    assert_eq!(edge_label["frequency"], "MULTIPLE");
    assert_eq!(edge_label["sort_keys"], json!(["mg_id"]));
}

#[tokio::test]
async fn incompatible_existing_schema_is_unsupported() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "exception": "class org.apache.hugegraph.exception.ExistedException",
            "message": "The property key 'mg_graph_id' has existed",
            "cause": ""
        })))
        .mount(&server)
        .await;
    let err = store_for(&server).ensure_schema().await.unwrap_err();
    assert!(matches!(err, GraphStoreError::Unsupported(_)), "{err}");
}

#[tokio::test]
async fn create_entity_sends_a_graph_scoped_typed_vertex() {
    let server = MockServer::start().await;
    mount_vertex_lookup(&server, json!([])).await;
    Mock::given(method("POST"))
        .and(path(format!("{ROOT}/graph/vertices/batch")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!(["id"])))
        .expect(1)
        .mount(&server)
        .await;

    let graph_id = GraphId::new();
    let entity = customer(graph_id);
    store_for(&server)
        .create_entity(entity.clone())
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let lookup = &requests[0];
    assert_eq!(
        lookup
            .url
            .query_pairs()
            .find(|(k, _)| k == "ids")
            .unwrap()
            .1,
        format!("\"{graph_id}:{}\"", entity.id()),
        "ids are JSON-quoted and graph-scoped"
    );
    let body: Value = requests[1].body_json().unwrap();
    assert_eq!(
        body,
        json!([{
            "id": format!("{graph_id}:{}", entity.id()),
            "label": "mindgraph_entity",
            "properties": {
                "mg_graph_id": graph_id.to_string(),
                "mg_type": "Customer",
                "mg_properties": r#"{"version":1,"properties":{"name":{"type":"string","value":"Customer A"}}}"#,
            }
        }])
    );
}

#[tokio::test]
async fn existing_vertex_is_a_conflict_and_nothing_is_written() {
    let server = MockServer::start().await;
    let entity = customer(GraphId::new());
    mount_vertex_lookup(&server, json!([vertex_json(&entity)])).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!([])))
        .expect(0)
        .mount(&server)
        .await;
    let err = store_for(&server).create_entity(entity).await.unwrap_err();
    assert!(matches!(err, GraphStoreError::Conflict(_)), "{err}");
}

#[tokio::test]
async fn get_entity_decodes_and_reports_missing_as_none() {
    let server = MockServer::start().await;
    let graph_id = GraphId::new();
    let entity = customer(graph_id);
    mount_vertex_lookup(&server, json!([vertex_json(&entity)])).await;
    let store = store_for(&server);
    assert_eq!(
        store.get_entity(graph_id, entity.id()).await.unwrap(),
        Some(entity)
    );

    let empty = MockServer::start().await;
    mount_vertex_lookup(&empty, json!([])).await;
    assert_eq!(
        store_for(&empty)
            .get_entity(graph_id, EntityId::new())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn corrupt_stored_data_is_reported_not_panicked() {
    let server = MockServer::start().await;
    let graph_id = GraphId::new();
    let entity = customer(graph_id);
    let mut vertex = vertex_json(&entity);
    vertex["properties"]["mg_properties"] =
        json!(r#"{"version":1,"properties":{"x":{"type":"uuid","value":"?"}}}"#);
    mount_vertex_lookup(&server, json!([vertex])).await;
    let err = store_for(&server)
        .get_entity(graph_id, entity.id())
        .await
        .unwrap_err();
    assert!(matches!(err, GraphStoreError::CorruptData(_)), "{err}");
}

#[tokio::test]
async fn backend_errors_are_categorised() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "exception": "class java.lang.RuntimeException",
            "message": "rocksdb failure",
        })))
        .mount(&server)
        .await;
    let err = store_for(&server)
        .get_entity(GraphId::new(), EntityId::new())
        .await
        .unwrap_err();
    assert!(matches!(err, GraphStoreError::BackendError { .. }), "{err}");
    assert!(
        err.to_string()
            .contains("RuntimeException: rocksdb failure")
    );

    let unavailable = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&unavailable)
        .await;
    let err = store_for(&unavailable)
        .get_entity(GraphId::new(), EntityId::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, GraphStoreError::BackendUnavailable { .. }),
        "{err}"
    );

    let garbage = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .mount(&garbage)
        .await;
    let err = store_for(&garbage)
        .get_entity(GraphId::new(), EntityId::new())
        .await
        .unwrap_err();
    assert!(matches!(err, GraphStoreError::BackendError { .. }), "{err}");
}

#[tokio::test]
async fn unreachable_and_slow_backends_are_unavailable() {
    // Reserve a free port, then close it so nothing is listening there.
    let address = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let store = HugeGraphStore::new(HugeGraphConfig::new(&address).unwrap()).unwrap();
    let err = store
        .get_entity(GraphId::new(), EntityId::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, GraphStoreError::BackendUnavailable { .. }),
        "{err}"
    );

    let slow = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "vertices": [] }))
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&slow)
        .await;
    let err = store_for(&slow)
        .get_entity(GraphId::new(), EntityId::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, GraphStoreError::BackendUnavailable { .. }),
        "{err}"
    );
}

#[tokio::test]
async fn clear_graph_deletes_only_vertices_of_that_graph() {
    let server = MockServer::start().await;
    let graph_id = GraphId::new();
    let (a, b) = (customer(graph_id), customer(graph_id));
    Mock::given(method("GET"))
        .and(path(format!("{ROOT}/graph/vertices")))
        .and(query_param("label", "mindgraph_entity"))
        .and(query_param(
            "properties",
            json!({ "mg_graph_id": graph_id.to_string() }).to_string(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vertices": [vertex_json(&a), vertex_json(&b)],
            "page": null
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(format!(
            r"^{ROOT}/graph/vertices/%22{graph_id}:[0-9a-f-]+%22$"
        )))
        .and(query_param("label", "mindgraph_entity"))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    // Any other DELETE (for example HugeGraph's whole-graph clear) fails the test.
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    store_for(&server).clear_graph(graph_id).await.unwrap();
}
