//! The behavioural contract of `MetadataStore`, shared by every backend.
//!
//! Each case receives a fresh, empty store. Backends opt in with
//! `metadata_store_contract!`, which generates one `#[tokio::test]` per case.

#![allow(clippy::unwrap_used)] // Panicking is the intended failure mode in tests.

use chrono::{DateTime, Duration, SubsecRound, TimeZone, Utc};
use mindgraph_core::{IngestionJob, JobError, JobStatus, MetadataStore, MetadataStoreError};
use mindgraph_domain::{
    DataSource, DataSourceId, DataSourceKind, DataSourceStatus, EntityType, EntityTypeId, GraphId,
    GraphProject, GraphStatus, IngestionJobId, Ontology, OntologyVersion, PropertiesSchema,
    PropertyDefinition, PropertyKey, PropertyType, RelationshipType, RelationshipTypeId, TypeName,
    WorkspaceId,
};
use mindgraph_ingestion::{
    DatasetProfile, EntityMapping, EntityMode, FieldMapping, FieldProfile, IngestionMapping,
    IngestionSummary,
};
use serde_json::json;

/// One test per case for `$factory`, an `async fn() -> Arc<dyn MetadataStore>`.
/// The bracket holds extra attributes, e.g. `[ignore = "needs PostgreSQL"]`.
macro_rules! metadata_store_contract {
    ($attrs:tt $factory:path) => {
        metadata_store_contract!(@cases $attrs $factory;
            local_workspace_is_idempotent,
            create_graph_stores_graph_and_initial_ontology,
            graph_fields_round_trip,
            missing_records_are_none,
            duplicate_graph_is_conflict,
            graph_in_unknown_workspace_is_not_found,
            initial_ontology_of_another_graph_is_rejected,
            list_graphs_is_newest_first_and_bounded,
            list_graphs_is_scoped_to_workspace,
            ontology_versions_are_append_only,
            ontology_for_missing_graph_is_not_found,
            set_active_ontology_switches_version,
            rich_ontology_round_trips,
            ontologies_are_isolated_per_graph,
            ping_succeeds,
            create_and_activate_ontology_is_atomic,
            activate_graph_moves_draft_to_active_only,
            data_sources_round_trip,
            data_sources_are_scoped_to_their_graph,
            data_source_writes_are_validated,
            data_source_profiles_follow_status,
            jobs_round_trip_with_pinned_ontology,
            job_creation_is_validated,
            job_transitions_are_controlled,
            job_progress_is_monotonic_and_bounded,
            idempotency_keys_are_unique_per_graph,
            incomplete_jobs_are_failed_on_recovery,
        );
    };
    (@cases $attrs:tt $factory:path; $($case:ident),* $(,)?) => {
        $( metadata_store_contract!(@case $attrs $factory; $case); )*
    };
    (@case [$($attr:meta),*] $factory:path; $case:ident) => {
        $(#[$attr])*
        #[tokio::test]
        async fn $case() {
            let store = $factory().await;
            contract::$case(store.as_ref()).await;
        }
    };
}

// --- fixtures ---------------------------------------------------------------

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn graph_at(workspace_id: WorkspaceId, name: &str, created_at: DateTime<Utc>) -> GraphProject {
    GraphProject::from_parts(
        GraphId::new(),
        workspace_id,
        name,
        None,
        GraphStatus::Draft,
        created_at,
        created_at,
    )
    .unwrap()
}

fn empty_ontology(graph_id: GraphId, version: u32) -> Ontology {
    Ontology::new(
        graph_id,
        OntologyVersion::new(version).unwrap(),
        vec![],
        vec![],
    )
    .unwrap()
}

async fn create(store: &dyn MetadataStore, graph: &GraphProject) {
    store
        .create_graph(graph, &empty_ontology(graph.id(), 1))
        .await
        .unwrap();
}

/// All six property types, required and optional, display names,
/// descriptions and relationship types.
fn rich_ontology(graph_id: GraphId, version: u32) -> Ontology {
    let key = |k: &str| PropertyKey::new(k).unwrap();
    let name = |n: &str| TypeName::new(n).unwrap();
    let customer = EntityType::new(
        EntityTypeId::new(),
        graph_id,
        name("Customer"),
        PropertiesSchema::new(vec![
            PropertyDefinition::required(key("customer_id"), PropertyType::String),
            PropertyDefinition::optional(key("age"), PropertyType::Integer),
            PropertyDefinition::optional(key("risk_score"), PropertyType::Float),
            PropertyDefinition::optional(key("verified"), PropertyType::Boolean),
            PropertyDefinition::required(key("joined_at"), PropertyType::DateTime),
            PropertyDefinition::optional(key("emails"), PropertyType::StringArray),
        ])
        .unwrap(),
    )
    .with_display_name("Bank Customer")
    .unwrap()
    .with_description("A retail customer")
    .unwrap();
    let account = EntityType::new(
        EntityTypeId::new(),
        graph_id,
        name("Account"),
        PropertiesSchema::default(),
    );
    let owns = RelationshipType::new(
        RelationshipTypeId::new(),
        graph_id,
        name("OWNS"),
        name("Customer"),
        name("Account"),
        PropertiesSchema::new(vec![PropertyDefinition::required(
            key("since"),
            PropertyType::DateTime,
        )])
        .unwrap(),
    );
    Ontology::new(
        graph_id,
        OntologyVersion::new(version).unwrap(),
        vec![customer, account],
        vec![owns],
    )
    .unwrap()
}

// --- cases ------------------------------------------------------------------

pub async fn local_workspace_is_idempotent(store: &dyn MetadataStore) {
    let first = store.ensure_local_workspace().await.unwrap();
    let second = store.ensure_local_workspace().await.unwrap();
    assert_eq!(first, second);
    assert_eq!(first.name(), "Local");
    assert_eq!(store.get_workspace(first.id()).await.unwrap(), Some(first));
}

pub async fn create_graph_stores_graph_and_initial_ontology(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let graph = graph_at(workspace.id(), "Fraud Investigation", now());
    create(store, &graph).await;

    assert_eq!(
        store.get_graph(graph.id()).await.unwrap(),
        Some(graph.clone())
    );
    let active = store
        .get_active_ontology(graph.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active, empty_ontology(graph.id(), 1));
    assert_eq!(
        store
            .get_ontology(graph.id(), OntologyVersion::INITIAL)
            .await
            .unwrap(),
        Some(active)
    );
}

pub async fn graph_fields_round_trip(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let created =
        Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap() + Duration::microseconds(123_456);
    for (status, description) in [
        (GraphStatus::Draft, None),
        (GraphStatus::Active, Some("Card fraud ring")),
        (GraphStatus::Archived, Some("Old")),
    ] {
        let graph = GraphProject::from_parts(
            GraphId::new(),
            workspace.id(),
            "Graph",
            description,
            status,
            created,
            created + Duration::hours(1),
        )
        .unwrap();
        create(store, &graph).await;
        assert_eq!(store.get_graph(graph.id()).await.unwrap(), Some(graph));
    }
}

pub async fn missing_records_are_none(store: &dyn MetadataStore) {
    let graph_id = GraphId::new();
    assert_eq!(store.get_workspace(WorkspaceId::new()).await.unwrap(), None);
    assert_eq!(store.get_graph(graph_id).await.unwrap(), None);
    assert_eq!(store.get_active_ontology(graph_id).await.unwrap(), None);
    assert_eq!(
        store
            .get_ontology(graph_id, OntologyVersion::INITIAL)
            .await
            .unwrap(),
        None
    );
}

pub async fn duplicate_graph_is_conflict(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let original = graph_at(workspace.id(), "Original", now());
    create(store, &original).await;
    let impostor = GraphProject::from_parts(
        original.id(),
        workspace.id(),
        "Impostor",
        None,
        GraphStatus::Active,
        now(),
        now(),
    )
    .unwrap();
    let err = store
        .create_graph(&impostor, &empty_ontology(original.id(), 1))
        .await
        .unwrap_err();
    assert!(matches!(err, MetadataStoreError::Conflict(_)), "{err}");
    assert_eq!(
        store.get_graph(original.id()).await.unwrap(),
        Some(original)
    );
}

pub async fn graph_in_unknown_workspace_is_not_found(store: &dyn MetadataStore) {
    store.ensure_local_workspace().await.unwrap();
    let orphan = graph_at(WorkspaceId::new(), "Orphan", now());
    let err = store
        .create_graph(&orphan, &empty_ontology(orphan.id(), 1))
        .await
        .unwrap_err();
    assert!(matches!(err, MetadataStoreError::NotFound(_)), "{err}");
    assert_eq!(store.get_graph(orphan.id()).await.unwrap(), None);
}

pub async fn initial_ontology_of_another_graph_is_rejected(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let graph = graph_at(workspace.id(), "Graph", now());
    let err = store
        .create_graph(&graph, &empty_ontology(GraphId::new(), 1))
        .await
        .unwrap_err();
    assert!(matches!(err, MetadataStoreError::InvalidInput(_)), "{err}");
    assert_eq!(store.get_graph(graph.id()).await.unwrap(), None);
}

pub async fn list_graphs_is_newest_first_and_bounded(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let base = Utc.with_ymd_and_hms(2026, 11, 3, 8, 0, 0).unwrap();
    let oldest = graph_at(workspace.id(), "oldest", base);
    let tie_a = graph_at(workspace.id(), "tie a", base + Duration::minutes(1));
    let tie_b = graph_at(workspace.id(), "tie b", base + Duration::minutes(1));
    let newest = graph_at(workspace.id(), "newest", base + Duration::minutes(2));
    for graph in [&tie_b, &oldest, &newest, &tie_a] {
        create(store, graph).await;
    }
    // Same created_at: the larger id comes first.
    let (tie_first, tie_second) = if tie_a.id() > tie_b.id() {
        (&tie_a, &tie_b)
    } else {
        (&tie_b, &tie_a)
    };
    let listed = store.list_graphs(workspace.id(), 10).await.unwrap();
    let ids: Vec<GraphId> = listed.iter().map(GraphProject::id).collect();
    assert_eq!(
        ids,
        vec![newest.id(), tie_first.id(), tie_second.id(), oldest.id()]
    );
    let top_two = store.list_graphs(workspace.id(), 2).await.unwrap();
    assert_eq!(top_two, listed[..2].to_vec());
}

pub async fn list_graphs_is_scoped_to_workspace(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    create(store, &graph_at(workspace.id(), "Mine", now())).await;
    assert!(
        store
            .list_graphs(WorkspaceId::new(), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.list_graphs(workspace.id(), 10).await.unwrap().len(),
        1
    );
}

pub async fn ontology_versions_are_append_only(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let graph = graph_at(workspace.id(), "Graph", now());
    create(store, &graph).await;

    let v2 = rich_ontology(graph.id(), 2);
    store.create_ontology(&v2).await.unwrap();
    for duplicate in [rich_ontology(graph.id(), 2), empty_ontology(graph.id(), 1)] {
        let err = store.create_ontology(&duplicate).await.unwrap_err();
        assert!(matches!(err, MetadataStoreError::Conflict(_)), "{err}");
    }
    assert_eq!(
        store.get_ontology(graph.id(), v2.version()).await.unwrap(),
        Some(v2)
    );
    assert_eq!(
        store
            .get_active_ontology(graph.id())
            .await
            .unwrap()
            .unwrap()
            .version(),
        OntologyVersion::INITIAL,
        "adding a version does not activate it"
    );
}

pub async fn ontology_for_missing_graph_is_not_found(store: &dyn MetadataStore) {
    let err = store
        .create_ontology(&empty_ontology(GraphId::new(), 2))
        .await
        .unwrap_err();
    assert!(matches!(err, MetadataStoreError::NotFound(_)), "{err}");
}

pub async fn set_active_ontology_switches_version(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let created = now() - Duration::minutes(5);
    let graph = graph_at(workspace.id(), "Graph", created);
    create(store, &graph).await;
    let v2 = rich_ontology(graph.id(), 2);
    store.create_ontology(&v2).await.unwrap();

    store
        .set_active_ontology(graph.id(), v2.version())
        .await
        .unwrap();
    assert_eq!(
        store.get_active_ontology(graph.id()).await.unwrap(),
        Some(v2)
    );
    let updated = store.get_graph(graph.id()).await.unwrap().unwrap();
    assert!(
        updated.updated_at() > graph.updated_at(),
        "activation touches updated_at"
    );
    assert_eq!(updated.created_at(), graph.created_at());

    let missing_version = OntologyVersion::new(9).unwrap();
    let err = store
        .set_active_ontology(graph.id(), missing_version)
        .await
        .unwrap_err();
    assert!(matches!(err, MetadataStoreError::NotFound(_)), "{err}");
    let err = store
        .set_active_ontology(GraphId::new(), OntologyVersion::INITIAL)
        .await
        .unwrap_err();
    assert!(matches!(err, MetadataStoreError::NotFound(_)), "{err}");
}

pub async fn rich_ontology_round_trips(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let graph = graph_at(workspace.id(), "Graph", now());
    let v1 = rich_ontology(graph.id(), 1);
    store.create_graph(&graph, &v1).await.unwrap();
    assert_eq!(
        store.get_active_ontology(graph.id()).await.unwrap(),
        Some(v1)
    );
}

pub async fn ontologies_are_isolated_per_graph(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let rich = graph_at(workspace.id(), "Rich", now());
    let empty = graph_at(workspace.id(), "Empty", now());
    store
        .create_graph(&rich, &rich_ontology(rich.id(), 1))
        .await
        .unwrap();
    create(store, &empty).await;
    let empty_active = store
        .get_active_ontology(empty.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(empty_active.graph_id(), empty.id());
    assert!(empty_active.entity_types().is_empty());
    assert_eq!(
        store
            .get_ontology(empty.id(), OntologyVersion::new(2).unwrap())
            .await
            .unwrap(),
        None
    );
}

pub async fn ping_succeeds(store: &dyn MetadataStore) {
    store.ping().await.unwrap();
}

// --- M4: ontology activation, data sources and jobs --------------------------

const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// A local-workspace graph with ontology v1, and a pending data source.
async fn graph_with_source(store: &dyn MetadataStore) -> (GraphProject, DataSource) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let graph = graph_at(workspace.id(), "Fraud", now());
    create(store, &graph).await;
    let source = data_source(graph.id(), "customers.csv");
    store
        .create_data_source(&source, &format!("sources/{}/source", source.id()))
        .await
        .unwrap();
    (graph, source)
}

fn data_source(graph_id: GraphId, filename: &str) -> DataSource {
    DataSource::new(
        DataSourceId::new(),
        graph_id,
        DataSourceKind::Csv,
        filename,
        81_232,
        SHA,
        now(),
    )
    .unwrap()
}

fn profile() -> DatasetProfile {
    DatasetProfile {
        record_count: 2,
        fields: vec![FieldProfile {
            name: "customer_id".into(),
            inferred_type: PropertyType::String,
            nullable: false,
            non_null_count: 2,
            sample_values: vec!["C001".into(), "C002".into()],
        }],
        sample_rows: vec![
            json!({"customer_id": "C001"}).as_object().unwrap().clone(),
            json!({"customer_id": "C002"}).as_object().unwrap().clone(),
        ],
    }
}

fn mapping() -> IngestionMapping {
    let field = || FieldMapping {
        source_field: "customer_id".into(),
        property: "customer_id".into(),
    };
    IngestionMapping {
        entities: vec![EntityMapping {
            alias: "customer".into(),
            entity_type: "Customer".into(),
            mode: EntityMode::CreateOrMatch,
            identity: vec![field()],
            properties: vec![field()],
        }],
        relationships: vec![],
    }
}

fn ingestion(graph_id: GraphId, source: DataSourceId, key: &str) -> IngestionJob {
    IngestionJob::ingestion(
        graph_id,
        source,
        OntologyVersion::INITIAL,
        key.to_owned(),
        mapping(),
        now(),
    )
}

pub async fn create_and_activate_ontology_is_atomic(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let graph = graph_at(workspace.id(), "Fraud", now());
    create(store, &graph).await;
    let v2 = rich_ontology(graph.id(), 2);
    store.create_and_activate_ontology(&v2).await.unwrap();
    assert_eq!(
        store.get_active_ontology(graph.id()).await.unwrap(),
        Some(v2.clone())
    );

    // A duplicate version stores nothing and leaves v2 active.
    let err = store
        .create_and_activate_ontology(&empty_ontology(graph.id(), 2))
        .await
        .unwrap_err();
    assert!(matches!(err, MetadataStoreError::Conflict(_)), "{err}");
    assert_eq!(
        store.get_active_ontology(graph.id()).await.unwrap(),
        Some(v2)
    );

    let err = store
        .create_and_activate_ontology(&empty_ontology(GraphId::new(), 2))
        .await
        .unwrap_err();
    assert!(matches!(err, MetadataStoreError::NotFound(_)), "{err}");
}

pub async fn activate_graph_moves_draft_to_active_only(store: &dyn MetadataStore) {
    let workspace = store.ensure_local_workspace().await.unwrap();
    let draft = graph_at(workspace.id(), "Draft", now());
    create(store, &draft).await;
    let active = store.activate_graph(draft.id()).await.unwrap();
    assert_eq!(active.status(), GraphStatus::Active);
    assert!(active.updated_at() >= draft.updated_at());
    assert_eq!(
        store.get_graph(draft.id()).await.unwrap(),
        Some(active.clone())
    );
    assert_eq!(
        store.activate_graph(draft.id()).await.unwrap().status(),
        GraphStatus::Active
    );

    let archived = GraphProject::from_parts(
        GraphId::new(),
        workspace.id(),
        "Archived",
        None,
        GraphStatus::Archived,
        now(),
        now(),
    )
    .unwrap();
    create(store, &archived).await;
    assert_eq!(
        store.activate_graph(archived.id()).await.unwrap().status(),
        GraphStatus::Archived,
        "archived graphs are not reactivated"
    );
    assert!(matches!(
        store.activate_graph(GraphId::new()).await,
        Err(MetadataStoreError::NotFound(_))
    ));
}

pub async fn data_sources_round_trip(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    let record = store
        .get_data_source(graph.id(), source.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.data_source, source);
    assert_eq!(record.data_source.status(), DataSourceStatus::Pending);
    assert_eq!(
        record.storage_key,
        format!("sources/{}/source", source.id())
    );
    assert!(record.profile.is_none());
    assert!(
        store
            .get_data_source(graph.id(), DataSourceId::new())
            .await
            .unwrap()
            .is_none()
    );
}

pub async fn data_sources_are_scoped_to_their_graph(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    let other = graph_at(graph.workspace_id(), "Other", now());
    create(store, &other).await;
    assert!(
        store
            .get_data_source(other.id(), source.id())
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        store
            .mark_data_source_ready(other.id(), source.id(), &profile())
            .await,
        Err(MetadataStoreError::NotFound(_))
    ));
    assert!(matches!(
        store.mark_data_source_failed(other.id(), source.id()).await,
        Err(MetadataStoreError::NotFound(_))
    ));
    let job = IngestionJob::analysis(other.id(), source.id(), now());
    assert!(matches!(
        store.create_job(&job).await,
        Err(MetadataStoreError::NotFound(_))
    ));
}

pub async fn data_source_writes_are_validated(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    assert!(matches!(
        store.create_data_source(&source, "x").await,
        Err(MetadataStoreError::Conflict(_))
    ));
    let orphan = data_source(GraphId::new(), "orphan.csv");
    assert!(matches!(
        store.create_data_source(&orphan, "x").await,
        Err(MetadataStoreError::NotFound(_))
    ));
    assert!(matches!(
        store
            .mark_data_source_ready(graph.id(), DataSourceId::new(), &profile())
            .await,
        Err(MetadataStoreError::NotFound(_))
    ));
}

pub async fn data_source_profiles_follow_status(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    let ready = store
        .mark_data_source_ready(graph.id(), source.id(), &profile())
        .await
        .unwrap();
    assert_eq!(ready.status(), DataSourceStatus::Ready);
    assert!(ready.updated_at() >= source.updated_at());
    assert_eq!(ready.created_at(), source.created_at());
    let record = store
        .get_data_source(graph.id(), source.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.profile, Some(profile()));
    assert_eq!(record.data_source, ready);

    let failed = store
        .mark_data_source_failed(graph.id(), source.id())
        .await
        .unwrap();
    assert_eq!(failed.status(), DataSourceStatus::Failed);
    let record = store
        .get_data_source(graph.id(), source.id())
        .await
        .unwrap()
        .unwrap();
    assert!(
        record.profile.is_none(),
        "a failed analysis keeps no profile"
    );
}

pub async fn jobs_round_trip_with_pinned_ontology(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    let analysis = IngestionJob::analysis(graph.id(), source.id(), now());
    let ingest = ingestion(graph.id(), source.id(), "key-1");
    store.create_job(&analysis).await.unwrap();
    store.create_job(&ingest).await.unwrap();
    assert_eq!(store.get_job(analysis.id).await.unwrap(), Some(analysis));
    let stored = store.get_job(ingest.id).await.unwrap().unwrap();
    assert_eq!(stored, ingest);
    assert_eq!(stored.ontology_version, Some(OntologyVersion::INITIAL));
    assert_eq!(stored.mapping, Some(mapping()));

    // Activating a newer version does not change what the job is pinned to.
    store
        .create_and_activate_ontology(&rich_ontology(graph.id(), 2))
        .await
        .unwrap();
    let stored = store.get_job(ingest.id).await.unwrap().unwrap();
    assert_eq!(stored.ontology_version, Some(OntologyVersion::INITIAL));
    assert!(
        store
            .get_job(IngestionJobId::new())
            .await
            .unwrap()
            .is_none()
    );
}

pub async fn job_creation_is_validated(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    let job = IngestionJob::analysis(graph.id(), source.id(), now());
    store.create_job(&job).await.unwrap();
    assert!(matches!(
        store.create_job(&job).await,
        Err(MetadataStoreError::Conflict(_))
    ));

    let missing_source = IngestionJob::analysis(graph.id(), DataSourceId::new(), now());
    assert!(matches!(
        store.create_job(&missing_source).await,
        Err(MetadataStoreError::NotFound(_))
    ));
    let mut unknown_version = ingestion(graph.id(), source.id(), "k");
    unknown_version.ontology_version = Some(OntologyVersion::new(9).unwrap());
    assert!(matches!(
        store.create_job(&unknown_version).await,
        Err(MetadataStoreError::InvalidInput(_))
    ));
    let mut running = IngestionJob::analysis(graph.id(), source.id(), now());
    running.status = JobStatus::Running;
    assert!(matches!(
        store.create_job(&running).await,
        Err(MetadataStoreError::InvalidInput(_))
    ));
    let mut unpinned = ingestion(graph.id(), source.id(), "k2");
    unpinned.ontology_version = None;
    assert!(matches!(
        store.create_job(&unpinned).await,
        Err(MetadataStoreError::InvalidInput(_))
    ));
}

pub async fn job_transitions_are_controlled(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    let job = ingestion(graph.id(), source.id(), "k");
    store.create_job(&job).await.unwrap();
    assert!(
        matches!(
            store.complete_job(job.id, None).await,
            Err(MetadataStoreError::Conflict(_))
        ),
        "queued jobs cannot complete"
    );

    let running = store.start_job(job.id).await.unwrap();
    assert_eq!(running.status, JobStatus::Running);
    assert!(running.updated_at >= job.updated_at);
    assert!(matches!(
        store.start_job(job.id).await,
        Err(MetadataStoreError::Conflict(_))
    ));

    let summary = IngestionSummary {
        records_processed: 1000,
        entities_created: 1000,
        entities_matched: 200,
        relationships_created: 980,
    };
    let done = store.complete_job(job.id, Some(&summary)).await.unwrap();
    assert_eq!(done.status, JobStatus::Completed);
    assert!((done.progress - 1.0).abs() < f64::EPSILON);
    assert_eq!(done.result, Some(summary));
    assert_eq!(store.get_job(job.id).await.unwrap(), Some(done));
    let error = JobError::new("X", "late");
    for refused in [
        store.start_job(job.id).await.map(drop),
        store.fail_job(job.id, &error).await.map(drop),
        store.complete_job(job.id, None).await.map(drop),
        store.update_job_progress(job.id, 0.5).await,
    ] {
        assert!(
            matches!(refused, Err(MetadataStoreError::Conflict(_))),
            "{refused:?}"
        );
    }

    // queued → failed and running → failed.
    let queued = IngestionJob::analysis(graph.id(), source.id(), now());
    store.create_job(&queued).await.unwrap();
    let failed = store
        .fail_job(queued.id, &JobError::new("QUEUE", "rejected"))
        .await
        .unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(failed.error, Some(JobError::new("QUEUE", "rejected")));
    let other = IngestionJob::analysis(graph.id(), source.id(), now());
    store.create_job(&other).await.unwrap();
    store.start_job(other.id).await.unwrap();
    store.update_job_progress(other.id, 0.4).await.unwrap();
    let failed = store.fail_job(other.id, &error).await.unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert!(
        (failed.progress - 0.4).abs() < f64::EPSILON,
        "progress is kept"
    );

    assert!(matches!(
        store.start_job(IngestionJobId::new()).await,
        Err(MetadataStoreError::NotFound(_))
    ));
}

pub async fn job_progress_is_monotonic_and_bounded(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    let job = IngestionJob::analysis(graph.id(), source.id(), now());
    store.create_job(&job).await.unwrap();
    assert!(
        matches!(
            store.update_job_progress(job.id, 0.1).await,
            Err(MetadataStoreError::Conflict(_))
        ),
        "only running jobs report progress"
    );
    store.start_job(job.id).await.unwrap();
    let progress = |expected: f64| async move {
        let stored = store.get_job(job.id).await.unwrap().unwrap().progress;
        assert!((stored - expected).abs() < 1e-9, "{stored} != {expected}");
    };
    store.update_job_progress(job.id, 0.42).await.unwrap();
    progress(0.42).await;
    store.update_job_progress(job.id, 0.2).await.unwrap();
    progress(0.42).await;
    store.update_job_progress(job.id, -3.0).await.unwrap();
    progress(0.42).await;
    store.update_job_progress(job.id, 7.0).await.unwrap();
    progress(1.0).await;
    assert!(matches!(
        store.update_job_progress(job.id, f64::NAN).await,
        Err(MetadataStoreError::InvalidInput(_))
    ));
}

pub async fn idempotency_keys_are_unique_per_graph(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    let first = ingestion(graph.id(), source.id(), "key-1");
    store.create_job(&first).await.unwrap();
    let duplicate = ingestion(graph.id(), source.id(), "key-1");
    assert!(matches!(
        store.create_job(&duplicate).await,
        Err(MetadataStoreError::Conflict(_))
    ));
    assert_eq!(
        store
            .find_job_by_idempotency_key(graph.id(), "key-1")
            .await
            .unwrap(),
        Some(first)
    );
    assert!(
        store
            .find_job_by_idempotency_key(graph.id(), "key-2")
            .await
            .unwrap()
            .is_none()
    );

    // The same key is independent in another graph.
    let other = graph_at(graph.workspace_id(), "Other", now());
    create(store, &other).await;
    let other_source = data_source(other.id(), "customers.csv");
    store.create_data_source(&other_source, "k").await.unwrap();
    let elsewhere = ingestion(other.id(), other_source.id(), "key-1");
    store.create_job(&elsewhere).await.unwrap();
    assert_eq!(
        store
            .find_job_by_idempotency_key(other.id(), "key-1")
            .await
            .unwrap()
            .map(|j| j.id),
        Some(elsewhere.id)
    );
}

pub async fn incomplete_jobs_are_failed_on_recovery(store: &dyn MetadataStore) {
    let (graph, source) = graph_with_source(store).await;
    let queued = IngestionJob::analysis(graph.id(), source.id(), now());
    let running = IngestionJob::analysis(graph.id(), source.id(), now());
    let completed = IngestionJob::analysis(graph.id(), source.id(), now());
    for job in [&queued, &running, &completed] {
        store.create_job(job).await.unwrap();
    }
    store.start_job(running.id).await.unwrap();
    store.start_job(completed.id).await.unwrap();
    store.complete_job(completed.id, None).await.unwrap();

    let error = JobError::new("PROCESS_RESTARTED", "restarted");
    assert_eq!(store.mark_incomplete_jobs_failed(&error).await.unwrap(), 2);
    for id in [queued.id, running.id] {
        let job = store.get_job(id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error, Some(error.clone()));
    }
    assert_eq!(
        store.get_job(completed.id).await.unwrap().unwrap().status,
        JobStatus::Completed
    );
    assert_eq!(store.mark_incomplete_jobs_failed(&error).await.unwrap(), 0);
}
