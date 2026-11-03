//! Live tests against PostgreSQL 17. Ignored by default, so
//! `cargo test --workspace` never needs a database. To run:
//!
//! ```bash
//! make postgres-test-up        # PostgreSQL 17 on 127.0.0.1:15432
//! make test-postgres
//! ```
//!
//! or point `MINDGRAPH_POSTGRES_TEST_URL` at any PostgreSQL 17 server whose
//! user may `CREATE DATABASE`. Every test creates its own fresh database
//! (`mindgraph_test_<uuid>`) and applies the migrations to it, so tests are
//! isolated from each other and from real data. Use a throwaway server: the
//! test databases are not dropped.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[macro_use]
mod contract;

use std::str::FromStr;
use std::sync::Arc;

use chrono::{SubsecRound, Utc};
use mindgraph_core::{IngestionJob, MetadataStore, MetadataStoreError};
use mindgraph_domain::{
    DataSource, DataSourceId, DataSourceKind, GraphId, GraphProject, GraphStatus, Ontology,
    OntologyVersion,
};
use mindgraph_ingestion::{EntityMapping, EntityMode, FieldMapping, IngestionMapping};
use mindgraph_metadata::PostgresMetadataStore;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::{AssertSqlSafe, Row};

const DEFAULT_TEST_URL: &str = "postgres://mindgraph:mindgraph@127.0.0.1:15432/mindgraph";

/// Creates an empty database, migrates it, and returns its pool.
async fn fresh_database() -> PgPool {
    let url = std::env::var("MINDGRAPH_POSTGRES_TEST_URL")
        .unwrap_or_else(|_| DEFAULT_TEST_URL.to_owned());
    let admin_options =
        PgConnectOptions::from_str(&url).expect("valid MINDGRAPH_POSTGRES_TEST_URL");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(admin_options.clone())
        .await
        .expect("PostgreSQL is reachable at MINDGRAPH_POSTGRES_TEST_URL");
    let name = format!("mindgraph_test_{}", uuid::Uuid::now_v7().simple());
    // The name is generated above from hex digits only.
    sqlx::query(AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(admin_options.database(&name))
        .await
        .unwrap();
    PostgresMetadataStore::from_pool(pool.clone())
        .migrate()
        .await
        .expect("migrations apply to a fresh database");
    pool
}

async fn postgres_store() -> Arc<dyn MetadataStore> {
    Arc::new(PostgresMetadataStore::from_pool(fresh_database().await))
}

metadata_store_contract!([ignore = "requires PostgreSQL 17; see module docs"] postgres_store);

fn draft(workspace: mindgraph_domain::WorkspaceId, name: &str) -> GraphProject {
    let now = Utc::now().trunc_subsecs(6);
    GraphProject::from_parts(
        GraphId::new(),
        workspace,
        name,
        None,
        GraphStatus::Draft,
        now,
        now,
    )
    .unwrap()
}

fn v1(graph_id: GraphId) -> Ontology {
    Ontology::new(graph_id, OntologyVersion::INITIAL, vec![], vec![]).unwrap()
}

async fn count(pool: &PgPool, sql: &'static str, id: uuid::Uuid) -> i64 {
    sqlx::query(sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
#[ignore = "requires PostgreSQL 17; see module docs"]
async fn migrations_create_every_table_and_are_idempotent() {
    let pool = fresh_database().await;
    PostgresMetadataStore::from_pool(pool.clone())
        .migrate()
        .await
        .unwrap();
    let tables: Vec<String> = sqlx::query(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name <> '_sqlx_migrations' ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|row| row.get(0))
    .collect();
    assert_eq!(
        tables,
        [
            "data_sources",
            "graph_projects",
            "ingestion_jobs",
            "ontology_versions",
            "saved_views",
            "workspaces"
        ]
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL 17; see module docs"]
async fn data_survives_a_reconnect_like_a_restart() {
    let pool = fresh_database().await;
    let first = PostgresMetadataStore::from_pool(pool.clone());
    let workspace = first.ensure_local_workspace().await.unwrap();
    let graph = draft(workspace.id(), "Persistent");
    first.create_graph(&graph, &v1(graph.id())).await.unwrap();

    // A new pool to the same database stands in for a restarted process.
    let options = pool.connect_options().as_ref().clone();
    pool.close().await;
    let second =
        PostgresMetadataStore::from_pool(PgPoolOptions::new().connect_with(options).await.unwrap());
    assert_eq!(second.ensure_local_workspace().await.unwrap(), workspace);
    assert_eq!(
        second.get_graph(graph.id()).await.unwrap(),
        Some(graph.clone())
    );
    assert!(
        second
            .get_active_ontology(graph.id())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL 17; see module docs"]
async fn concurrent_bootstraps_create_one_local_workspace() {
    let pool = fresh_database().await;
    let store = Arc::new(PostgresMetadataStore::from_pool(pool.clone()));
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.ensure_local_workspace().await.unwrap().id() })
        })
        .collect();
    let mut ids = Vec::new();
    for task in tasks {
        ids.push(task.await.unwrap());
    }
    ids.dedup();
    assert_eq!(ids.len(), 1);
    let rows: i64 = sqlx::query("SELECT count(*) FROM workspaces")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL 17; see module docs"]
async fn graph_creation_rolls_back_when_the_ontology_insert_fails() {
    let pool = fresh_database().await;
    let store = PostgresMetadataStore::from_pool(pool.clone());
    let workspace = store.ensure_local_workspace().await.unwrap();
    // This database is private to the test, so the trigger affects nothing else.
    sqlx::query(
        "CREATE FUNCTION fail_ontology_insert() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'simulated failure'; END; $$",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_ontology_insert BEFORE INSERT ON ontology_versions \
         FOR EACH ROW EXECUTE FUNCTION fail_ontology_insert()",
    )
    .execute(&pool)
    .await
    .unwrap();

    let graph = draft(workspace.id(), "Doomed");
    let err = store
        .create_graph(&graph, &v1(graph.id()))
        .await
        .unwrap_err();
    assert!(
        matches!(err, MetadataStoreError::BackendError { .. }),
        "{err}"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM graph_projects WHERE id = $1",
            graph.id().as_uuid()
        )
        .await,
        0,
        "the graph row was rolled back"
    );
    assert_eq!(store.get_graph(graph.id()).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires PostgreSQL 17; see module docs"]
async fn corrupt_ontology_json_is_reported_as_corrupt_data() {
    let pool = fresh_database().await;
    let store = PostgresMetadataStore::from_pool(pool.clone());
    let workspace = store.ensure_local_workspace().await.unwrap();
    let graph = draft(workspace.id(), "Graph");
    store.create_graph(&graph, &v1(graph.id())).await.unwrap();
    for (version, document) in [
        (
            2,
            r#"{"format": 1, "entity_types": "nope", "relationship_types": []}"#,
        ),
        (
            3,
            r#"{"format": 99, "entity_types": [], "relationship_types": []}"#,
        ),
        (
            4,
            r#"{"format": 1, "entity_types": [], "relationship_types": [{"id": "x", "name": "OWNS", "source_entity_type": "A", "target_entity_type": "B", "properties": []}]}"#,
        ),
    ] {
        sqlx::query("INSERT INTO ontology_versions (graph_id, version, schema_json) VALUES ($1, $2, $3::jsonb)")
            .bind(graph.id().as_uuid())
            .bind(version)
            .bind(document)
            .execute(&pool)
            .await
            .unwrap();
        let version = OntologyVersion::new(u32::try_from(version).unwrap()).unwrap();
        let err = store.get_ontology(graph.id(), version).await.unwrap_err();
        assert!(matches!(err, MetadataStoreError::CorruptData(_)), "{err}");
    }
    store
        .set_active_ontology(graph.id(), OntologyVersion::new(2).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        store.get_active_ontology(graph.id()).await,
        Err(MetadataStoreError::CorruptData(_))
    ));
}

#[tokio::test]
#[ignore = "requires PostgreSQL 17; see module docs"]
async fn database_constraints_protect_integrity() {
    let pool = fresh_database().await;
    let store = PostgresMetadataStore::from_pool(pool.clone());
    let workspace = store.ensure_local_workspace().await.unwrap();
    let graph = draft(workspace.id(), "Graph");
    store.create_graph(&graph, &v1(graph.id())).await.unwrap();
    let insert_graph = "INSERT INTO graph_projects \
        (id, workspace_id, name, status, active_ontology_version, created_at, updated_at) \
        VALUES ($1, $2, $3, $4, $5, now(), now())";

    // Unknown status.
    let bad_status = sqlx::query(insert_graph)
        .bind(GraphId::new().as_uuid())
        .bind(workspace.id().as_uuid())
        .bind("Bad")
        .bind("deleted")
        .bind(1_i32)
        .execute(&pool)
        .await;
    assert!(bad_status.is_err(), "status is constrained");

    // A graph whose active ontology does not exist fails at commit.
    let dangling = sqlx::query(insert_graph)
        .bind(GraphId::new().as_uuid())
        .bind(workspace.id().as_uuid())
        .bind("Dangling")
        .bind("draft")
        .bind(1_i32)
        .execute(&pool)
        .await;
    assert!(dangling.is_err(), "active ontology version must exist");

    // Version 0.
    let zero = sqlx::query(
        "INSERT INTO ontology_versions (graph_id, version, schema_json) VALUES ($1, 0, '{}')",
    )
    .bind(graph.id().as_uuid())
    .execute(&pool)
    .await;
    assert!(zero.is_err(), "versions start at 1");

    // Ontology versions are immutable.
    let update = sqlx::query("UPDATE ontology_versions SET schema_json = '{}' WHERE graph_id = $1")
        .bind(graph.id().as_uuid())
        .execute(&pool)
        .await;
    let message = update.unwrap_err().to_string();
    assert!(message.contains("immutable"), "{message}");
}

/// A graph (with ontology v1) and one pending data source.
async fn graph_and_source(store: &PostgresMetadataStore) -> (GraphProject, DataSource) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let graph = draft(workspace.id(), "Graph");
    store.create_graph(&graph, &v1(graph.id())).await.unwrap();
    let source = DataSource::new(
        DataSourceId::new(),
        graph.id(),
        DataSourceKind::Csv,
        "customers.csv",
        10,
        &"a".repeat(64),
        Utc::now().trunc_subsecs(6),
    )
    .unwrap();
    store
        .create_data_source(&source, "sources/x/source")
        .await
        .unwrap();
    (graph, source)
}

fn ingestion_job(graph: &GraphProject, source: &DataSource, key: &str) -> IngestionJob {
    let field = || FieldMapping {
        source_field: "customer_id".into(),
        property: "customer_id".into(),
    };
    IngestionJob::ingestion(
        graph.id(),
        source.id(),
        OntologyVersion::INITIAL,
        key.to_owned(),
        IngestionMapping {
            entities: vec![EntityMapping {
                alias: "customer".into(),
                entity_type: "Customer".into(),
                mode: EntityMode::CreateOrMatch,
                identity: vec![field()],
                properties: vec![field()],
            }],
            relationships: vec![],
        },
        Utc::now().trunc_subsecs(6),
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL 17; see module docs"]
async fn m4_constraints_protect_data_sources_and_jobs() {
    let pool = fresh_database().await;
    let store = PostgresMetadataStore::from_pool(pool.clone());
    let (graph, source) = graph_and_source(&store).await;

    // A ready data source must have a profile.
    let ready_without_profile =
        sqlx::query("UPDATE data_sources SET status = 'ready' WHERE id = $1")
            .bind(source.id().as_uuid())
            .execute(&pool)
            .await;
    assert!(ready_without_profile.is_err());

    // Checksums are 64 lowercase hex digits.
    let bad_checksum = sqlx::query("UPDATE data_sources SET content_sha256 = 'XYZ' WHERE id = $1")
        .bind(source.id().as_uuid())
        .execute(&pool)
        .await;
    assert!(bad_checksum.is_err());

    let job = ingestion_job(&graph, &source, "k");
    store.create_job(&job).await.unwrap();
    for (sql, what) in [
        (
            "UPDATE ingestion_jobs SET status = 'failed' WHERE id = $1",
            "failed needs an error",
        ),
        (
            "UPDATE ingestion_jobs SET status = 'completed' WHERE id = $1",
            "completed needs progress 1",
        ),
        (
            "UPDATE ingestion_jobs SET ontology_version = NULL WHERE id = $1",
            "ingest jobs are pinned",
        ),
        (
            "UPDATE ingestion_jobs SET ontology_version = 7 WHERE id = $1",
            "the pinned version exists",
        ),
        (
            "UPDATE ingestion_jobs SET kind = 'export' WHERE id = $1",
            "kind is constrained",
        ),
        (
            "UPDATE ingestion_jobs SET result_json = '{}' WHERE id = $1",
            "results only when completed",
        ),
    ] {
        let result = sqlx::query(sql).bind(job.id.as_uuid()).execute(&pool).await;
        assert!(result.is_err(), "{what}");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL 17; see module docs"]
async fn corrupt_job_json_is_reported_as_corrupt_data() {
    let pool = fresh_database().await;
    let store = PostgresMetadataStore::from_pool(pool.clone());
    let (graph, source) = graph_and_source(&store).await;
    let job = ingestion_job(&graph, &source, "k");
    store.create_job(&job).await.unwrap();
    sqlx::query("UPDATE ingestion_jobs SET mapping_json = '{\"entities\": 3}' WHERE id = $1")
        .bind(job.id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        store.get_job(job.id).await,
        Err(MetadataStoreError::CorruptData(_))
    ));
}

#[tokio::test]
#[ignore = "requires PostgreSQL 17; see module docs"]
async fn concurrent_jobs_with_one_idempotency_key_store_one_job() {
    let pool = fresh_database().await;
    let store = Arc::new(PostgresMetadataStore::from_pool(pool.clone()));
    let (graph, source) = graph_and_source(&store).await;
    let attempts: Vec<_> = (0..8)
        .map(|_| {
            let store = Arc::clone(&store);
            let job = ingestion_job(&graph, &source, "same-key");
            tokio::spawn(async move { store.create_job(&job).await })
        })
        .collect();
    let mut created = 0;
    for attempt in attempts {
        match attempt.await.unwrap() {
            Ok(()) => created += 1,
            Err(MetadataStoreError::Conflict(_)) => {}
            Err(other) => panic!("{other}"),
        }
    }
    assert_eq!(created, 1);
    let rows = count(
        &pool,
        "SELECT count(*) FROM ingestion_jobs WHERE graph_id = $1",
        graph.id().as_uuid(),
    )
    .await;
    assert_eq!(rows, 1);
}
