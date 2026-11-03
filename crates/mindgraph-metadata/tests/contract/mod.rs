//! The behavioural contract of `MetadataStore`, shared by every backend.
//!
//! Each case receives a fresh, empty store. Backends opt in with
//! `metadata_store_contract!`, which generates one `#[tokio::test]` per case.

#![allow(clippy::unwrap_used)] // Panicking is the intended failure mode in tests.

use chrono::{DateTime, Duration, SubsecRound, TimeZone, Utc};
use mindgraph_core::{MetadataStore, MetadataStoreError};
use mindgraph_domain::{
    EntityType, EntityTypeId, GraphId, GraphProject, GraphStatus, Ontology, OntologyVersion,
    PropertiesSchema, PropertyDefinition, PropertyKey, PropertyType, RelationshipType,
    RelationshipTypeId, TypeName, WorkspaceId,
};

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
