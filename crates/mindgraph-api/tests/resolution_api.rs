//! Entity resolution HTTP endpoints (M9) over the in-memory stores: the full
//! review flow, every error code, the stable error envelope and request id
//! propagation. Offline: no PostgreSQL, no HugeGraph, no model.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

mod resolution_flow;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use mindgraph_domain::{EntityId, GraphId};
use mindgraph_graph::{GraphStore, MemoryGraphStore};
use resolution_flow::{TestApp, app, assert_error, get, post, scan_body, send};
use serde_json::{Value, json};

async fn memory_app() -> (TestApp, Arc<MemoryGraphStore>) {
    let store = Arc::new(MemoryGraphStore::new());
    (app(store.clone() as Arc<dyn GraphStore>).await, store)
}

/// Runs a scan and returns the id of its first pending candidate.
async fn pending_candidate(app: &TestApp) -> String {
    let graph = app.graph_id;
    let scan = post(
        app,
        &format!("/api/v1/graphs/{graph}/resolution/scans"),
        &scan_body(),
    )
    .await;
    assert_eq!(scan.status, StatusCode::CREATED, "{}", scan.body);
    let list = get(
        app,
        &format!("/api/v1/graphs/{graph}/resolution/candidates"),
    )
    .await;
    list.body["data"][0]["id"].as_str().unwrap().to_owned()
}

fn decisions_uri(app: &TestApp, candidate: &str) -> String {
    format!(
        "/api/v1/graphs/{}/resolution/candidates/{candidate}/decisions",
        app.graph_id
    )
}

#[tokio::test]
async fn the_review_flow_works_and_leaves_the_graph_untouched() {
    let (app, store) = memory_app().await;
    resolution_flow::run(&app, store.as_ref()).await;
}

#[tokio::test]
async fn scans_are_listed_and_read_back() {
    let (app, _) = memory_app().await;
    let graph = app.graph_id;
    let scan = post(
        &app,
        &format!("/api/v1/graphs/{graph}/resolution/scans"),
        &scan_body(),
    )
    .await;
    assert_eq!(scan.headers["x-request-id"], "client-request-1");
    let id = scan.body["scan_id"].as_str().unwrap();
    let read = get(
        &app,
        &format!("/api/v1/graphs/{graph}/resolution/scans/{id}"),
    )
    .await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body, scan.body);
    assert_eq!(read.body["rules"][3]["kind"], "fuzzy_name");
    assert_eq!(read.body["rules"][3]["threshold"], 0.9);
    assert_eq!(read.body["rules"][2]["country"], "ID");
    assert_eq!(read.body["comparison_budget"], 20_000);
    assert_eq!(read.body["skipped"]["already_pending"], 0);
    let list = get(
        &app,
        &format!("/api/v1/graphs/{graph}/resolution/scans?limit=5"),
    )
    .await;
    assert_eq!(list.body["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scan_requests_are_validated() {
    let (app, _) = memory_app().await;
    let graph = app.graph_id;
    let uri = format!("/api/v1/graphs/{graph}/resolution/scans");
    let with = |change: Value| {
        let mut body = scan_body();
        for (key, value) in change.as_object().unwrap() {
            body[key] = value.clone();
        }
        body
    };
    let cases = [
        (
            with(json!({"entity_type": "Planet"})),
            StatusCode::BAD_REQUEST,
            "UNKNOWN_ENTITY_TYPE",
        ),
        (
            with(json!({"rules": [{"property": "unknown", "kind": "fuzzy_name"}]})),
            StatusCode::BAD_REQUEST,
            "INVALID_RULE",
        ),
        (
            with(json!({"rules": [{"property": "name", "kind": "fuzzy_name", "threshold": 1.5}]})),
            StatusCode::BAD_REQUEST,
            "INVALID_RULE",
        ),
        (
            with(json!({"rules": [{"property": "age", "kind": "normalized_email"}]})),
            StatusCode::BAD_REQUEST,
            "INVALID_RULE",
        ),
        (
            with(
                json!({"rules": [{"property": "phone", "kind": "normalized_phone", "country": "XX"}]}),
            ),
            StatusCode::BAD_REQUEST,
            "INVALID_RULE",
        ),
        (
            with(json!({"rules": [{"property": "name", "kind": "soundex"}]})),
            StatusCode::BAD_REQUEST,
            "INVALID_RULE",
        ),
        (
            with(json!({"rules": []})),
            StatusCode::BAD_REQUEST,
            "INVALID_RULE",
        ),
        (
            with(json!({"max_entities": 1000})),
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
        ),
        (
            with(json!({"unexpected": true})),
            StatusCode::BAD_REQUEST,
            "INVALID_JSON",
        ),
    ];
    for (body, status, code) in cases {
        let reply = post(&app, &uri, &body).await;
        assert_error(&reply, status, code);
    }
    let too_many: Vec<Value> = (0..11)
        .map(|_| json!({"property": "name", "kind": "fuzzy_name"}))
        .collect();
    assert_error(
        &post(&app, &uri, &with(json!({ "rules": too_many }))).await,
        StatusCode::BAD_REQUEST,
        "INVALID_RULE",
    );
    // Request size bound: a scan request is a few rules, never data.
    let huge = json!({
        "entity_type": "Customer",
        "rules": [{"property": "x".repeat(20_000), "kind": "fuzzy_name"}]
    });
    let reply = post(&app, &uri, &huge).await;
    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );

    assert_error(
        &post(
            &app,
            "/api/v1/graphs/not-a-uuid/resolution/scans",
            &scan_body(),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_GRAPH_ID",
    );
    assert_error(
        &post(
            &app,
            &format!("/api/v1/graphs/{}/resolution/scans", GraphId::new()),
            &scan_body(),
        )
        .await,
        StatusCode::NOT_FOUND,
        "GRAPH_NOT_FOUND",
    );
    assert_error(
        &get(
            &app,
            &format!("/api/v1/graphs/{graph}/resolution/scans/{}", GraphId::new()),
        )
        .await,
        StatusCode::NOT_FOUND,
        "SCAN_NOT_FOUND",
    );
    assert_error(
        &get(
            &app,
            &format!("/api/v1/graphs/{graph}/resolution/scans/nope"),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_SCAN_ID",
    );
}

#[tokio::test]
async fn candidate_listing_is_bounded_and_validated() {
    let (app, _) = memory_app().await;
    pending_candidate(&app).await;
    let graph = app.graph_id;
    let base = format!("/api/v1/graphs/{graph}/resolution/candidates");
    let page = get(&app, &format!("{base}?limit=1&status=all")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.body["limit"], 1);
    assert_eq!(page.body["next_cursor"], Value::Null);
    let by_entity = get(&app, &format!("{base}?entity_id={}", app.ikhwan_a)).await;
    assert_eq!(by_entity.body["data"].as_array().unwrap().len(), 1);
    let none = get(&app, &format!("{base}?entity_id={}", app.alice)).await;
    assert_eq!(none.body["data"].as_array().unwrap().len(), 0);

    for (query, status, code) in [
        ("status=maybe", StatusCode::BAD_REQUEST, "INVALID_QUERY"),
        ("limit=101", StatusCode::BAD_REQUEST, "INVALID_REQUEST"),
        ("limit=0", StatusCode::BAD_REQUEST, "INVALID_REQUEST"),
        ("cursor=nope", StatusCode::BAD_REQUEST, "INVALID_CURSOR"),
        (
            "entity_id=nope",
            StatusCode::BAD_REQUEST,
            "INVALID_ENTITY_ID",
        ),
        ("scan_id=nope", StatusCode::BAD_REQUEST, "INVALID_SCAN_ID"),
        ("unknown=1", StatusCode::BAD_REQUEST, "INVALID_QUERY"),
    ] {
        assert_error(&get(&app, &format!("{base}?{query}")).await, status, code);
    }
    assert_error(
        &get(&app, &format!("{base}?cursor={}", EntityId::new())).await,
        StatusCode::BAD_REQUEST,
        "INVALID_CURSOR",
    );
}

#[tokio::test]
async fn decisions_reject_conflicts_and_replay_repeats() {
    let (app, _) = memory_app().await;
    let candidate = pending_candidate(&app).await;
    let uri = decisions_uri(&app, &candidate);

    assert_error(
        &post(&app, &uri, &json!({"decision": "merge"})).await,
        StatusCode::BAD_REQUEST,
        "INVALID_DECISION",
    );
    assert_error(
        &post(&app, &uri, &json!({"decision": "same_entity"})).await,
        StatusCode::BAD_REQUEST,
        "INVALID_DECISION",
    );
    assert_error(
        &post(
            &app,
            &uri,
            &json!({"decision": "different_entities", "canonical_entity_id": app.ikhwan_a.to_string()}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_DECISION",
    );
    assert_error(
        &post(
            &app,
            &uri,
            &json!({"decision": "same_entity", "canonical_entity_id": app.alice.to_string()}),
        )
        .await,
        StatusCode::CONFLICT,
        "IDENTITY_CONFLICT",
    );
    assert_error(
        &post(
            &app,
            &uri,
            &json!({"decision": "same_entity", "canonical_entity_id": "nope"}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_ENTITY_ID",
    );

    let first = post(&app, &uri, &json!({"decision": "different_entities"})).await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    assert_eq!(first.body["candidate"]["status"], "rejected");
    assert_eq!(first.body["identity"]["resolved"], false);
    let repeat = post(&app, &uri, &json!({"decision": "different_entities"})).await;
    assert_eq!(repeat.status, StatusCode::OK);
    assert_eq!(repeat.body["replayed"], true);
    assert_eq!(repeat.body["decision"]["id"], first.body["decision"]["id"]);

    let change =
        json!({"decision": "same_entity", "canonical_entity_id": app.ikhwan_a.to_string()});
    assert_error(
        &post(&app, &uri, &change).await,
        StatusCode::CONFLICT,
        "DECISION_CONFLICT",
    );
    let mut reconsider = change.clone();
    reconsider["reconsider"] = json!(true);
    let reviewed = post(&app, &uri, &reconsider).await;
    assert_eq!(reviewed.status, StatusCode::CREATED, "{}", reviewed.body);
    assert_eq!(reviewed.body["decision"]["reconsidered"], true);
    let detail = get(
        &app,
        &format!(
            "/api/v1/graphs/{}/resolution/candidates/{candidate}",
            app.graph_id
        ),
    )
    .await;
    assert_eq!(detail.body["decisions"].as_array().unwrap().len(), 2);
    assert_eq!(detail.body["identity_a"]["resolved"], true);
}

#[tokio::test]
async fn candidates_and_identities_are_graph_scoped() {
    let (app, _) = memory_app().await;
    let candidate = pending_candidate(&app).await;
    let other = app.other_graph;
    assert_error(
        &get(
            &app,
            &format!("/api/v1/graphs/{other}/resolution/candidates/{candidate}"),
        )
        .await,
        StatusCode::NOT_FOUND,
        "CANDIDATE_NOT_FOUND",
    );
    assert_error(
        &post(
            &app,
            &format!("/api/v1/graphs/{other}/resolution/candidates/{candidate}/decisions"),
            &json!({"decision": "different_entities"}),
        )
        .await,
        StatusCode::NOT_FOUND,
        "CANDIDATE_NOT_FOUND",
    );
    assert_error(
        &get(
            &app,
            &format!(
                "/api/v1/graphs/{}/resolution/candidates/{}",
                app.graph_id,
                GraphId::new()
            ),
        )
        .await,
        StatusCode::NOT_FOUND,
        "CANDIDATE_NOT_FOUND",
    );
    assert_error(
        &get(
            &app,
            &format!("/api/v1/graphs/{}/resolution/candidates/nope", app.graph_id),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_CANDIDATE_ID",
    );
    // An entity of graph A is not found in graph B.
    assert_error(
        &get(
            &app,
            &format!(
                "/api/v1/graphs/{other}/resolution/entities/{}",
                app.ikhwan_a
            ),
        )
        .await,
        StatusCode::NOT_FOUND,
        "ENTITY_NOT_FOUND",
    );
    assert_error(
        &get(
            &app,
            &format!("/api/v1/graphs/{}/resolution/entities/nope", app.graph_id),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_ENTITY_ID",
    );
    let alone = get(
        &app,
        &format!(
            "/api/v1/graphs/{}/resolution/entities/{}",
            app.graph_id, app.alice
        ),
    )
    .await;
    assert_eq!(alone.status, StatusCode::OK);
    assert_eq!(alone.body["canonical_entity_id"], app.alice.to_string());
    assert_eq!(alone.body["is_canonical"], true);
    assert_eq!(alone.body["resolved"], false);
    assert_eq!(alone.body["resolution_type"], Value::Null);
    assert_eq!(
        alone.body["member_entity_ids"],
        json!([app.alice.to_string()])
    );
    assert_eq!(alone.body["physical_graph_merged"], false);
}

#[tokio::test]
async fn a_generated_request_id_is_returned_when_none_is_sent() {
    let (app, _) = memory_app().await;
    let request = Request::get(format!(
        "/api/v1/graphs/{}/resolution/candidates",
        app.graph_id
    ))
    .body(Body::empty())
    .unwrap();
    let reply = send(&app, request).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(!reply.headers["x-request-id"].is_empty());
    let request = Request::post(format!("/api/v1/graphs/{}/resolution/scans", app.graph_id))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("{}"))
        .unwrap();
    let reply = send(&app, request).await;
    assert_eq!(reply.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(reply.body["error"]["code"], "UNSUPPORTED_MEDIA_TYPE");
}
