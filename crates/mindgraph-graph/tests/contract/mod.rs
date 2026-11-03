//! The behavioural contract of `GraphStore`, shared by every backend.
//!
//! Each case takes a `&dyn GraphStore` and works in fresh, random graph ids,
//! so the suite can run against a shared live server. Backends opt in with
//! `graph_store_contract!`, which generates one `#[tokio::test]` per case.

#![allow(clippy::unwrap_used)] // Panicking is the intended failure mode in tests.

use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};
use mindgraph_domain::{
    Entity, EntityId, GraphId, Properties, PropertyKey, PropertyValue, Relationship,
    RelationshipId, TypeName,
};
use mindgraph_graph::{
    Direction, EntitySearch, GraphStore, GraphStoreError, NeighborQuery, PathQuery,
};

/// Generates one test per contract case for `$factory`, an
/// `async fn() -> Arc<dyn GraphStore>`. The bracket holds extra attributes
/// for every generated test, e.g. `[ignore = "needs a server"]`.
macro_rules! graph_store_contract {
    ($attrs:tt $factory:path) => {
        graph_store_contract!(@cases $attrs $factory;
            create_and_get_entity,
            missing_entity_is_none,
            duplicate_entity_is_conflict,
            same_entity_id_in_two_graphs_is_independent,
            batch_create_entities,
            mixed_graph_batch_is_rejected,
            duplicate_inside_batch_is_conflict,
            empty_batches_are_noops,
            created_relationship_is_readable,
            missing_source_is_rejected,
            missing_target_is_rejected,
            cross_graph_relationship_is_rejected,
            self_loop_is_accepted,
            duplicate_relationship_is_conflict,
            parallel_relationships_are_kept,
            search_filters_by_entity_type,
            search_filters_by_exact_property,
            search_respects_limit_and_orders_by_id,
            search_pages_through_large_types_exactly_once,
            search_matches_any_of_several_identities,
            search_is_isolated_per_graph,
            neighbors_follow_direction,
            neighbors_filter_by_relationship_type,
            neighbors_respect_limit,
            neighbors_handle_cycles_and_self_loops,
            neighbors_of_missing_entity_is_not_found,
            neighbors_are_isolated_per_graph,
            path_direct,
            path_multi_hop_is_shortest,
            path_none_when_disconnected,
            path_respects_direction,
            path_respects_max_depth,
            path_is_cycle_safe,
            path_to_self,
            path_uses_smallest_relationship_id,
            path_is_isolated_per_graph,
            path_with_missing_endpoint_is_not_found,
            subgraph_returns_requested_entities_and_internal_relationships,
            subgraph_ignores_missing_and_repeated_ids,
            subgraph_is_isolated_per_graph,
            delete_removes_entity_and_incident_relationships,
            delete_missing_entity_returns_false,
            delete_is_isolated_per_graph,
            clear_graph_leaves_other_graphs_untouched,
        );
    };
    (@cases $attrs:tt $factory:path; $($case:ident),* $(,)?) => {
        $( graph_store_contract!(@case $attrs $factory; $case); )*
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

fn name(value: &str) -> TypeName {
    TypeName::new(value).unwrap()
}

fn key(value: &str) -> PropertyKey {
    PropertyKey::new(value).unwrap()
}

fn entity(graph_id: GraphId, entity_type: &str, properties: &[(&str, PropertyValue)]) -> Entity {
    let mut props = Properties::new();
    for (k, v) in properties {
        props.insert(key(k), v.clone()).unwrap();
    }
    Entity::new(EntityId::new(), graph_id, name(entity_type), props)
}

fn node(graph_id: GraphId) -> Entity {
    entity(graph_id, "Node", &[])
}

/// A copy of `original` (same id and type) in another graph.
fn twin(original: &Entity, graph_id: GraphId) -> Entity {
    Entity::new(
        original.id(),
        graph_id,
        original.entity_type().clone(),
        Properties::new(),
    )
}

fn link(relationship_type: &str, source: &Entity, target: &Entity) -> Relationship {
    link_with_id(RelationshipId::new(), relationship_type, source, target)
}

fn link_with_id(
    id: RelationshipId,
    relationship_type: &str,
    source: &Entity,
    target: &Entity,
) -> Relationship {
    Relationship::new(
        id,
        source.graph_id(),
        name(relationship_type),
        source.id(),
        target.id(),
        Properties::new(),
    )
}

fn ids<'a>(entities: impl IntoIterator<Item = &'a Entity>) -> Vec<EntityId> {
    entities.into_iter().map(Entity::id).collect()
}

fn rel_ids<'a>(relationships: impl IntoIterator<Item = &'a Relationship>) -> Vec<RelationshipId> {
    relationships.into_iter().map(Relationship::id).collect()
}

fn sorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
    items.sort();
    items
}

async fn seed(store: &dyn GraphStore, entities: &[&Entity], relationships: &[&Relationship]) {
    store
        .create_entities(entities.iter().map(|e| (*e).clone()).collect())
        .await
        .unwrap();
    store
        .create_relationships(relationships.iter().map(|r| (*r).clone()).collect())
        .await
        .unwrap();
}

async fn cleanup(store: &dyn GraphStore, graphs: &[GraphId]) {
    for graph_id in graphs {
        store.clear_graph(*graph_id).await.unwrap();
    }
}

fn depth(direction: Direction, max_depth: usize) -> PathQuery {
    PathQuery::new(direction).with_max_depth(max_depth).unwrap()
}

// --- entities ----------------------------------------------------------------

pub async fn create_and_get_entity(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let joined = Utc.with_ymd_and_hms(2026, 11, 3, 8, 30, 0).unwrap();
    let customer = entity(
        graph_id,
        "Customer",
        &[
            ("name", "Ikhwan Nasution".into()),
            ("age", 34_i64.into()),
            ("risk_score", 0.91.into()),
            ("verified", true.into()),
            ("joined_at", joined.into()),
            ("emails", vec!["ikhwan@example.com".to_owned()].into()),
        ],
    );
    store.create_entity(customer.clone()).await.unwrap();
    assert_eq!(
        store.get_entity(graph_id, customer.id()).await.unwrap(),
        Some(customer)
    );
    cleanup(store, &[graph_id]).await;
}

pub async fn missing_entity_is_none(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    assert_eq!(
        store.get_entity(graph_id, EntityId::new()).await.unwrap(),
        None
    );
}

pub async fn duplicate_entity_is_conflict(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let original = entity(graph_id, "Customer", &[("name", "Original".into())]);
    store.create_entity(original.clone()).await.unwrap();

    let impostor = Entity::new(original.id(), graph_id, name("Device"), Properties::new());
    let err = store.create_entity(impostor).await.unwrap_err();
    assert!(matches!(err, GraphStoreError::Conflict(_)), "{err}");
    assert_eq!(
        store.get_entity(graph_id, original.id()).await.unwrap(),
        Some(original),
        "the existing entity must not be overwritten"
    );
    cleanup(store, &[graph_id]).await;
}

pub async fn same_entity_id_in_two_graphs_is_independent(store: &dyn GraphStore) {
    let (graph_a, graph_b) = (GraphId::new(), GraphId::new());
    let in_a = entity(graph_a, "Customer", &[("name", "A".into())]);
    let in_b = Entity::new(in_a.id(), graph_b, name("Device"), Properties::new());
    store.create_entity(in_a.clone()).await.unwrap();
    store.create_entity(in_b.clone()).await.unwrap();
    assert_eq!(
        store.get_entity(graph_a, in_a.id()).await.unwrap(),
        Some(in_a)
    );
    assert_eq!(
        store.get_entity(graph_b, in_b.id()).await.unwrap(),
        Some(in_b)
    );
    cleanup(store, &[graph_a, graph_b]).await;
}

pub async fn batch_create_entities(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let batch: Vec<Entity> = (0..25).map(|_| node(graph_id)).collect();
    store.create_entities(batch.clone()).await.unwrap();
    for entity in &batch {
        assert_eq!(
            store
                .get_entity(graph_id, entity.id())
                .await
                .unwrap()
                .as_ref(),
            Some(entity)
        );
    }
    cleanup(store, &[graph_id]).await;
}

pub async fn mixed_graph_batch_is_rejected(store: &dyn GraphStore) {
    let (graph_a, graph_b) = (GraphId::new(), GraphId::new());
    let (a, b) = (node(graph_a), node(graph_b));
    let err = store
        .create_entities(vec![a.clone(), b.clone()])
        .await
        .unwrap_err();
    assert!(matches!(err, GraphStoreError::InvalidInput(_)), "{err}");
    assert_eq!(store.get_entity(graph_a, a.id()).await.unwrap(), None);
    assert_eq!(store.get_entity(graph_b, b.id()).await.unwrap(), None);
}

pub async fn duplicate_inside_batch_is_conflict(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let a = node(graph_id);
    let err = store
        .create_entities(vec![a.clone(), node(graph_id), a.clone()])
        .await
        .unwrap_err();
    assert!(matches!(err, GraphStoreError::Conflict(_)), "{err}");
    assert_eq!(
        store.get_entity(graph_id, a.id()).await.unwrap(),
        None,
        "a rejected batch writes nothing"
    );
}

pub async fn empty_batches_are_noops(store: &dyn GraphStore) {
    store.create_entities(Vec::new()).await.unwrap();
    store.create_relationships(Vec::new()).await.unwrap();
}

// --- relationships -------------------------------------------------------------

pub async fn created_relationship_is_readable(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (customer, account) = (node(graph_id), node(graph_id));
    let mut props = Properties::new();
    props
        .insert(
            key("since"),
            Utc.with_ymd_and_hms(2020, 1, 2, 3, 4, 5).unwrap(),
        )
        .unwrap();
    props.insert(key("share"), 0.5).unwrap();
    let owns = Relationship::new(
        RelationshipId::new(),
        graph_id,
        name("OWNS"),
        customer.id(),
        account.id(),
        props,
    );
    seed(store, &[&customer, &account], &[]).await;
    store.create_relationship(owns.clone()).await.unwrap();

    let neighborhood = store
        .get_neighbors(
            graph_id,
            customer.id(),
            &NeighborQuery::new(Direction::Outgoing),
        )
        .await
        .unwrap();
    assert_eq!(neighborhood.relationships(), &[owns]);
    cleanup(store, &[graph_id]).await;
}

pub async fn missing_source_is_rejected(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (target, ghost) = (node(graph_id), node(graph_id));
    seed(store, &[&target], &[]).await;
    let err = store
        .create_relationship(link("OWNS", &ghost, &target))
        .await
        .unwrap_err();
    assert!(
        matches!(err, GraphStoreError::EntityNotFound { entity_id, .. } if entity_id == ghost.id()),
        "{err}"
    );
    assert_eq!(store.get_entity(graph_id, ghost.id()).await.unwrap(), None);
    cleanup(store, &[graph_id]).await;
}

pub async fn missing_target_is_rejected(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (source, ghost) = (node(graph_id), node(graph_id));
    seed(store, &[&source], &[]).await;
    let err = store
        .create_relationship(link("OWNS", &source, &ghost))
        .await
        .unwrap_err();
    assert!(
        matches!(err, GraphStoreError::EntityNotFound { entity_id, .. } if entity_id == ghost.id()),
        "{err}"
    );
    cleanup(store, &[graph_id]).await;
}

pub async fn cross_graph_relationship_is_rejected(store: &dyn GraphStore) {
    let (graph_a, graph_b) = (GraphId::new(), GraphId::new());
    let (in_a, in_b) = (node(graph_a), node(graph_b));
    seed(store, &[&in_a], &[]).await;
    seed(store, &[&in_b], &[]).await;
    // Declared in graph A but pointing at an entity that only exists in B.
    let cross = Relationship::new(
        RelationshipId::new(),
        graph_a,
        name("KNOWS"),
        in_a.id(),
        in_b.id(),
        Properties::new(),
    );
    let err = store.create_relationship(cross).await.unwrap_err();
    assert!(
        matches!(err, GraphStoreError::EntityNotFound { .. }),
        "{err}"
    );
    let neighborhood = store
        .get_neighbors(graph_a, in_a.id(), &NeighborQuery::default())
        .await
        .unwrap();
    assert!(neighborhood.relationships().is_empty());
    cleanup(store, &[graph_a, graph_b]).await;
}

pub async fn self_loop_is_accepted(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let account = node(graph_id);
    let loop_ = link("TRANSFERRED_TO", &account, &account);
    seed(store, &[&account], &[&loop_]).await;
    let neighborhood = store
        .get_neighbors(graph_id, account.id(), &NeighborQuery::default())
        .await
        .unwrap();
    assert_eq!(neighborhood.entities(), &[account]);
    assert_eq!(neighborhood.relationships(), &[loop_]);
    cleanup(store, &[graph_id]).await;
}

pub async fn duplicate_relationship_is_conflict(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (a, b, c) = (node(graph_id), node(graph_id), node(graph_id));
    let ab = link("LINKS", &a, &b);
    seed(store, &[&a, &b, &c], &[&ab]).await;

    // Same id, different endpoints: still a conflict.
    let reused = link_with_id(ab.id(), "LINKS", &b, &c);
    let err = store.create_relationship(reused).await.unwrap_err();
    assert!(matches!(err, GraphStoreError::Conflict(_)), "{err}");
    let err = store
        .create_relationships(vec![link("LINKS", &b, &c), ab.clone()])
        .await
        .unwrap_err();
    assert!(matches!(err, GraphStoreError::Conflict(_)), "{err}");
    let inside = link("LINKS", &a, &c);
    let err = store
        .create_relationships(vec![inside.clone(), inside])
        .await
        .unwrap_err();
    assert!(matches!(err, GraphStoreError::Conflict(_)), "{err}");

    let subgraph = store
        .get_subgraph(graph_id, &ids([&a, &b, &c]))
        .await
        .unwrap();
    assert_eq!(
        subgraph.relationships(),
        &[ab],
        "rejected writes store nothing"
    );
    cleanup(store, &[graph_id]).await;
}

pub async fn parallel_relationships_are_kept(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (a, b) = (node(graph_id), node(graph_id));
    let first = link("SENT", &a, &b);
    let second = link("SENT", &a, &b);
    seed(store, &[&a, &b], &[&first, &second]).await;
    let neighborhood = store
        .get_neighbors(graph_id, a.id(), &NeighborQuery::new(Direction::Outgoing))
        .await
        .unwrap();
    assert_eq!(
        rel_ids(neighborhood.relationships()),
        vec![first.id(), second.id()]
    );
    cleanup(store, &[graph_id]).await;
}

// --- search --------------------------------------------------------------------

pub async fn search_filters_by_entity_type(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let c1 = entity(graph_id, "Customer", &[]);
    let device = entity(graph_id, "Device", &[]);
    let c2 = entity(graph_id, "Customer", &[]);
    seed(store, &[&c1, &device, &c2], &[]).await;
    let found = store
        .search_entities(
            graph_id,
            &EntitySearch::new().with_entity_type(name("Customer")),
        )
        .await
        .unwrap();
    assert_eq!(ids(&found), vec![c1.id(), c2.id()]);
    let all = store
        .search_entities(graph_id, &EntitySearch::new())
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    cleanup(store, &[graph_id]).await;
}

pub async fn search_filters_by_exact_property(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let indonesia = entity(
        graph_id,
        "Customer",
        &[("country", "Indonesia".into()), ("tier", 2_i64.into())],
    );
    let malaysia = entity(graph_id, "Customer", &[("country", "Malaysia".into())]);
    let device = entity(graph_id, "Device", &[("country", "Indonesia".into())]);
    seed(store, &[&indonesia, &malaysia, &device], &[]).await;

    let search = EntitySearch::new()
        .with_entity_type(name("Customer"))
        .with_property(key("country"), "Indonesia");
    let found = store.search_entities(graph_id, &search).await.unwrap();
    assert_eq!(found, vec![indonesia.clone()]);

    let by_integer = EntitySearch::new().with_property(key("tier"), 2_i64);
    assert_eq!(
        ids(&store.search_entities(graph_id, &by_integer).await.unwrap()),
        vec![indonesia.id()]
    );
    // Exact means exact type too: 2.0 (float) does not match 2 (integer).
    let by_float = EntitySearch::new().with_property(key("tier"), 2.0);
    assert!(
        store
            .search_entities(graph_id, &by_float)
            .await
            .unwrap()
            .is_empty()
    );
    cleanup(store, &[graph_id]).await;
}

pub async fn search_respects_limit_and_orders_by_id(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let mut created: Vec<Entity> = (0..5).map(|_| node(graph_id)).collect();
    // Insert in reverse to show ordering comes from ids, not insertion.
    let reversed: Vec<&Entity> = created.iter().rev().collect();
    seed(store, &reversed, &[]).await;
    created.sort_by_key(Entity::id);

    let first_three = store
        .search_entities(graph_id, &EntitySearch::new().with_limit(3).unwrap())
        .await
        .unwrap();
    assert_eq!(ids(&first_three), ids(&created[..3]));
    let again = store
        .search_entities(graph_id, &EntitySearch::new().with_limit(3).unwrap())
        .await
        .unwrap();
    assert_eq!(first_three, again, "deterministic");
    cleanup(store, &[graph_id]).await;
}

/// More entities than one backend page (HugeGraph pages hold 500): every
/// entity is found exactly once, including those at page boundaries.
pub async fn search_pages_through_large_types_exactly_once(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let created: Vec<Entity> = (0..1_234)
        .map(|i| {
            entity(
                graph_id,
                "Customer",
                &[("customer_id", PropertyValue::from(format!("C{i:05}")))],
            )
        })
        .collect();
    for chunk in created.chunks(250) {
        store.create_entities(chunk.to_vec()).await.unwrap();
    }
    let mut sorted_ids = ids(&created);
    sorted_ids.sort();

    let by_type = EntitySearch::new()
        .with_entity_type(name("Customer"))
        .with_limit(EntitySearch::MAX_LIMIT)
        .unwrap();
    let found = store.search_entities(graph_id, &by_type).await.unwrap();
    assert_eq!(
        ids(&found),
        sorted_ids[..EntitySearch::MAX_LIMIT],
        "no duplicates, no gaps"
    );

    for index in [0, 499, 500, 501, 999, 1_000, 1_233] {
        let search = EntitySearch::new()
            .with_entity_type(name("Customer"))
            .with_property(
                key("customer_id"),
                PropertyValue::from(format!("C{index:05}")),
            )
            .with_limit(2)
            .unwrap();
        let matches = store.search_entities(graph_id, &search).await.unwrap();
        assert_eq!(ids(&matches), [created[index].id()], "customer {index}");
    }
    cleanup(store, &[graph_id]).await;
}

/// One search for a batch of identities, as dataset ingestion does.
pub async fn search_matches_any_of_several_identities(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let customer = |id: &str, region: &str| {
        entity(
            graph_id,
            "Customer",
            &[
                ("customer_id", PropertyValue::from(id)),
                ("region", PropertyValue::from(region)),
            ],
        )
    };
    let (c1, c2, c3) = (
        customer("C1", "EU"),
        customer("C2", "US"),
        customer("C3", "EU"),
    );
    let other_type = entity(
        graph_id,
        "Account",
        &[("customer_id", PropertyValue::from("C1"))],
    );
    seed(store, &[&c1, &c2, &c3, &other_type], &[]).await;
    let identity = |pairs: &[(&str, &str)]| -> BTreeMap<PropertyKey, PropertyValue> {
        pairs
            .iter()
            .map(|(k, v)| (key(k), PropertyValue::from(*v)))
            .collect()
    };
    let search = |alternatives| {
        EntitySearch::new()
            .with_entity_type(name("Customer"))
            .with_any_of(alternatives)
            .unwrap()
    };

    let found = store
        .search_entities(
            graph_id,
            &search(vec![
                identity(&[("customer_id", "C1")]),
                identity(&[("customer_id", "C3")]),
                identity(&[("customer_id", "C9")]),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(ids(&found), sorted(vec![c1.id(), c3.id()]));

    // Composite alternatives must match completely.
    let composite = search(vec![
        identity(&[("customer_id", "C2"), ("region", "EU")]),
        identity(&[("customer_id", "C3"), ("region", "EU")]),
    ]);
    let found = store.search_entities(graph_id, &composite).await.unwrap();
    assert_eq!(ids(&found), [c3.id()]);

    // Alternatives AND plain property filters.
    let narrowed = search(vec![
        identity(&[("customer_id", "C1")]),
        identity(&[("customer_id", "C2")]),
    ])
    .with_property(key("region"), "US");
    let found = store.search_entities(graph_id, &narrowed).await.unwrap();
    assert_eq!(ids(&found), [c2.id()]);

    for invalid in [
        EntitySearch::new().with_any_of(vec![]),
        EntitySearch::new().with_any_of(vec![BTreeMap::new()]),
        EntitySearch::new().with_any_of(vec![identity(&[("a", "b")]); 1_001]),
    ] {
        assert!(matches!(invalid, Err(GraphStoreError::InvalidInput(_))));
    }
    cleanup(store, &[graph_id]).await;
}

pub async fn search_is_isolated_per_graph(store: &dyn GraphStore) {
    let (graph_a, graph_b) = (GraphId::new(), GraphId::new());
    let in_a = entity(graph_a, "Customer", &[("country", "Indonesia".into())]);
    let in_b = entity(graph_b, "Customer", &[("country", "Indonesia".into())]);
    seed(store, &[&in_a], &[]).await;
    seed(store, &[&in_b], &[]).await;
    let search = EntitySearch::new().with_property(key("country"), "Indonesia");
    assert_eq!(
        store.search_entities(graph_a, &search).await.unwrap(),
        vec![in_a]
    );
    assert_eq!(
        store.search_entities(graph_b, &search).await.unwrap(),
        vec![in_b]
    );
    assert!(
        store
            .search_entities(GraphId::new(), &search)
            .await
            .unwrap()
            .is_empty()
    );
    cleanup(store, &[graph_a, graph_b]).await;
}

// --- neighbors -------------------------------------------------------------------

pub async fn neighbors_follow_direction(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (hub, out_node, in_node) = (node(graph_id), node(graph_id), node(graph_id));
    let outgoing = link("OWNS", &hub, &out_node);
    let incoming = link("OWNS", &in_node, &hub);
    seed(store, &[&hub, &out_node, &in_node], &[&outgoing, &incoming]).await;

    let out = store
        .get_neighbors(graph_id, hub.id(), &NeighborQuery::new(Direction::Outgoing))
        .await
        .unwrap();
    assert_eq!(rel_ids(out.relationships()), vec![outgoing.id()]);
    assert_eq!(ids(out.entities()), sorted(vec![hub.id(), out_node.id()]));

    let inc = store
        .get_neighbors(graph_id, hub.id(), &NeighborQuery::new(Direction::Incoming))
        .await
        .unwrap();
    assert_eq!(rel_ids(inc.relationships()), vec![incoming.id()]);
    assert_eq!(ids(inc.entities()), sorted(vec![hub.id(), in_node.id()]));

    let both = store
        .get_neighbors(graph_id, hub.id(), &NeighborQuery::new(Direction::Both))
        .await
        .unwrap();
    assert_eq!(
        rel_ids(both.relationships()),
        sorted(vec![outgoing.id(), incoming.id()])
    );
    assert_eq!(both.entities().len(), 3, "source entity is included once");
    cleanup(store, &[graph_id]).await;
}

pub async fn neighbors_filter_by_relationship_type(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (customer, account, device) = (node(graph_id), node(graph_id), node(graph_id));
    let owns = link("OWNS", &customer, &account);
    let uses = link("USES", &customer, &device);
    seed(store, &[&customer, &account, &device], &[&owns, &uses]).await;
    let query = NeighborQuery::default().with_relationship_type(name("USES"));
    let neighborhood = store
        .get_neighbors(graph_id, customer.id(), &query)
        .await
        .unwrap();
    assert_eq!(neighborhood.relationships(), &[uses]);
    assert_eq!(
        ids(neighborhood.entities()),
        sorted(vec![customer.id(), device.id()])
    );
    cleanup(store, &[graph_id]).await;
}

pub async fn neighbors_respect_limit(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let hub = node(graph_id);
    let leaves: Vec<Entity> = (0..4).map(|_| node(graph_id)).collect();
    let links: Vec<Relationship> = leaves
        .iter()
        .map(|leaf| link("LINKS", &hub, leaf))
        .collect();
    let mut entities: Vec<&Entity> = leaves.iter().collect();
    entities.push(&hub);
    seed(store, &entities, &links.iter().collect::<Vec<_>>()).await;

    let query = NeighborQuery::new(Direction::Outgoing)
        .with_limit(2)
        .unwrap();
    let neighborhood = store
        .get_neighbors(graph_id, hub.id(), &query)
        .await
        .unwrap();
    let expected = sorted(rel_ids(&links));
    assert_eq!(
        rel_ids(neighborhood.relationships()),
        expected[..2].to_vec()
    );
    assert_eq!(neighborhood.entities().len(), 3);
    cleanup(store, &[graph_id]).await;
}

pub async fn neighbors_handle_cycles_and_self_loops(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (a, b) = (node(graph_id), node(graph_id));
    let ab = link("LINKS", &a, &b);
    let ba = link("LINKS", &b, &a);
    let aa = link("LINKS", &a, &a);
    seed(store, &[&a, &b], &[&ab, &ba, &aa]).await;
    let neighborhood = store
        .get_neighbors(graph_id, a.id(), &NeighborQuery::new(Direction::Both))
        .await
        .unwrap();
    assert_eq!(
        rel_ids(neighborhood.relationships()),
        sorted(vec![ab.id(), ba.id(), aa.id()]),
        "each relationship appears once"
    );
    assert_eq!(ids(neighborhood.entities()), sorted(vec![a.id(), b.id()]));
    cleanup(store, &[graph_id]).await;
}

pub async fn neighbors_of_missing_entity_is_not_found(store: &dyn GraphStore) {
    let err = store
        .get_neighbors(GraphId::new(), EntityId::new(), &NeighborQuery::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, GraphStoreError::EntityNotFound { .. }),
        "{err}"
    );
}

pub async fn neighbors_are_isolated_per_graph(store: &dyn GraphStore) {
    let (graph_a, graph_b) = (GraphId::new(), GraphId::new());
    let (hub_a, leaf_a) = (node(graph_a), node(graph_a));
    let hub_b = twin(&hub_a, graph_b);
    let leaf_b = node(graph_b);
    let in_a = link("LINKS", &hub_a, &leaf_a);
    let in_b = link("LINKS", &hub_b, &leaf_b);
    seed(store, &[&hub_a, &leaf_a], &[&in_a]).await;
    seed(store, &[&hub_b, &leaf_b], &[&in_b]).await;

    let a = store
        .get_neighbors(graph_a, hub_a.id(), &NeighborQuery::default())
        .await
        .unwrap();
    assert_eq!(a.relationships(), &[in_a]);
    assert!(a.entities().iter().all(|e| e.graph_id() == graph_a));
    let b = store
        .get_neighbors(graph_b, hub_b.id(), &NeighborQuery::default())
        .await
        .unwrap();
    assert_eq!(b.relationships(), &[in_b]);
    assert!(b.entities().iter().all(|e| e.graph_id() == graph_b));
    cleanup(store, &[graph_a, graph_b]).await;
}

// --- paths -----------------------------------------------------------------------

pub async fn path_direct(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (customer, account) = (node(graph_id), node(graph_id));
    let owns = link("OWNS", &customer, &account);
    seed(store, &[&customer, &account], &[&owns]).await;
    let path = store
        .find_path(graph_id, customer.id(), account.id(), &PathQuery::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(path.entities(), &[customer, account]);
    assert_eq!(path.relationships(), &[owns]);
    cleanup(store, &[graph_id]).await;
}

pub async fn path_multi_hop_is_shortest(store: &dyn GraphStore) {
    // a → b → c → d and the shortcut a → x → d.
    let graph_id = GraphId::new();
    let (a, b, c, d, x) = (
        node(graph_id),
        node(graph_id),
        node(graph_id),
        node(graph_id),
        node(graph_id),
    );
    let long = [link("L", &a, &b), link("L", &b, &c), link("L", &c, &d)];
    let ax = link("L", &a, &x);
    let xd = link("L", &x, &d);
    let mut relationships: Vec<&Relationship> = long.iter().collect();
    relationships.extend([&ax, &xd]);
    seed(store, &[&a, &b, &c, &d, &x], &relationships).await;

    let path = store
        .find_path(graph_id, a.id(), d.id(), &depth(Direction::Outgoing, 6))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(path.hops(), 2);
    assert_eq!(ids(path.entities()), vec![a.id(), x.id(), d.id()]);
    assert_eq!(rel_ids(path.relationships()), vec![ax.id(), xd.id()]);
    cleanup(store, &[graph_id]).await;
}

pub async fn path_none_when_disconnected(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (a, b, island) = (node(graph_id), node(graph_id), node(graph_id));
    let ab = link("L", &a, &b);
    seed(store, &[&a, &b, &island], &[&ab]).await;
    let path = store
        .find_path(graph_id, a.id(), island.id(), &depth(Direction::Both, 6))
        .await
        .unwrap();
    assert_eq!(path, None);
    cleanup(store, &[graph_id]).await;
}

pub async fn path_respects_direction(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (a, b) = (node(graph_id), node(graph_id));
    let ab = link("L", &a, &b);
    seed(store, &[&a, &b], &[&ab]).await;
    let (outgoing, incoming, both) = (
        PathQuery::new(Direction::Outgoing),
        PathQuery::new(Direction::Incoming),
        PathQuery::new(Direction::Both),
    );
    let backwards = |query| store.find_path(graph_id, b.id(), a.id(), query);
    assert!(backwards(&outgoing).await.unwrap().is_none());
    let path = backwards(&incoming).await.unwrap().unwrap();
    assert_eq!(ids(path.entities()), vec![b.id(), a.id()]);
    assert_eq!(path.relationships(), std::slice::from_ref(&ab));
    assert!(backwards(&both).await.unwrap().is_some());
    cleanup(store, &[graph_id]).await;
}

pub async fn path_respects_max_depth(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let chain: Vec<Entity> = (0..4).map(|_| node(graph_id)).collect();
    let links: Vec<Relationship> = chain.windows(2).map(|w| link("L", &w[0], &w[1])).collect();
    seed(
        store,
        &chain.iter().collect::<Vec<_>>(),
        &links.iter().collect::<Vec<_>>(),
    )
    .await;
    let (first, last) = (chain[0].id(), chain[3].id());
    let shallow = store
        .find_path(graph_id, first, last, &depth(Direction::Outgoing, 2))
        .await
        .unwrap();
    assert_eq!(shallow, None, "three hops needed, two allowed");
    let deep = store
        .find_path(graph_id, first, last, &depth(Direction::Outgoing, 3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deep.hops(), 3);
    cleanup(store, &[graph_id]).await;
}

pub async fn path_is_cycle_safe(store: &dyn GraphStore) {
    // a → b → c → a (cycle), c → d.
    let graph_id = GraphId::new();
    let (a, b, c, d) = (
        node(graph_id),
        node(graph_id),
        node(graph_id),
        node(graph_id),
    );
    let relationships = [
        link("L", &a, &b),
        link("L", &b, &c),
        link("L", &c, &a),
        link("L", &c, &d),
        link("L", &b, &b),
    ];
    seed(
        store,
        &[&a, &b, &c, &d],
        &relationships.iter().collect::<Vec<_>>(),
    )
    .await;
    let path = store
        .find_path(graph_id, a.id(), d.id(), &depth(Direction::Outgoing, 6))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ids(path.entities()), vec![a.id(), b.id(), c.id(), d.id()]);
    let unreachable = store
        .find_path(graph_id, d.id(), a.id(), &depth(Direction::Outgoing, 6))
        .await
        .unwrap();
    assert_eq!(unreachable, None);
    cleanup(store, &[graph_id]).await;
}

pub async fn path_to_self(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let a = node(graph_id);
    seed(store, &[&a], &[]).await;
    let path = store
        .find_path(graph_id, a.id(), a.id(), &PathQuery::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(path.entities(), &[a]);
    assert_eq!(path.hops(), 0);
    cleanup(store, &[graph_id]).await;
}

pub async fn path_uses_smallest_relationship_id(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (a, b, c) = (node(graph_id), node(graph_id), node(graph_id));
    let (r1, r2, r3, r4) = (
        RelationshipId::new(),
        RelationshipId::new(),
        RelationshipId::new(),
        RelationshipId::new(),
    );
    // Parallel relationships on both hops, inserted in a scrambled order.
    let relationships = [
        link_with_id(r2, "L", &a, &b),
        link_with_id(r4, "L", &b, &c),
        link_with_id(r1, "L", &a, &b),
        link_with_id(r3, "L", &b, &c),
    ];
    seed(
        store,
        &[&a, &b, &c],
        &relationships.iter().collect::<Vec<_>>(),
    )
    .await;
    for direction in [Direction::Outgoing, Direction::Both] {
        let query = depth(direction, 3);
        let first = store
            .find_path(graph_id, a.id(), c.id(), &query)
            .await
            .unwrap();
        let second = store
            .find_path(graph_id, a.id(), c.id(), &query)
            .await
            .unwrap();
        let path = first.unwrap();
        assert_eq!(rel_ids(path.relationships()), vec![r1, r3], "{direction:?}");
        assert_eq!(Some(path), second, "deterministic");
    }
    let incoming = store
        .find_path(graph_id, c.id(), a.id(), &depth(Direction::Incoming, 3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rel_ids(incoming.relationships()), vec![r3, r1]);
    cleanup(store, &[graph_id]).await;
}

pub async fn path_is_isolated_per_graph(store: &dyn GraphStore) {
    let (graph_a, graph_b) = (GraphId::new(), GraphId::new());
    let (a, b, c) = (node(graph_a), node(graph_a), node(graph_a));
    let links = [link("L", &a, &b), link("L", &b, &c)];
    seed(store, &[&a, &b, &c], &links.iter().collect::<Vec<_>>()).await;
    // Graph B has entities with the same ids as a and c, but no relationships.
    seed(store, &[&twin(&a, graph_b), &twin(&c, graph_b)], &[]).await;

    assert!(
        store
            .find_path(graph_a, a.id(), c.id(), &PathQuery::default())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store
            .find_path(graph_b, a.id(), c.id(), &PathQuery::default())
            .await
            .unwrap(),
        None
    );
    cleanup(store, &[graph_a, graph_b]).await;
}

pub async fn path_with_missing_endpoint_is_not_found(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let a = node(graph_id);
    seed(store, &[&a], &[]).await;
    let ghost = EntityId::new();
    for (source, target) in [(a.id(), ghost), (ghost, a.id())] {
        let err = store
            .find_path(graph_id, source, target, &PathQuery::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, GraphStoreError::EntityNotFound { entity_id, .. } if entity_id == ghost),
            "{err}"
        );
    }
    cleanup(store, &[graph_id]).await;
}

// --- subgraph --------------------------------------------------------------------

pub async fn subgraph_returns_requested_entities_and_internal_relationships(
    store: &dyn GraphStore,
) {
    let graph_id = GraphId::new();
    let (a, b, c, outside) = (
        node(graph_id),
        node(graph_id),
        node(graph_id),
        node(graph_id),
    );
    let ab = link("L", &a, &b);
    let bc = link("L", &b, &c);
    let ca = link("L", &c, &a);
    let a_out = link("L", &a, &outside);
    let bb = link("L", &b, &b);
    seed(
        store,
        &[&a, &b, &c, &outside],
        &[&ab, &bc, &ca, &a_out, &bb],
    )
    .await;

    let subgraph = store
        .get_subgraph(graph_id, &ids([&a, &b, &c]))
        .await
        .unwrap();
    assert_eq!(ids(subgraph.entities()), sorted(ids([&a, &b, &c])));
    assert_eq!(
        rel_ids(subgraph.relationships()),
        sorted(rel_ids([&ab, &bc, &ca, &bb])),
        "no relationship to an entity outside the request"
    );
    cleanup(store, &[graph_id]).await;
}

pub async fn subgraph_ignores_missing_and_repeated_ids(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (a, b) = (node(graph_id), node(graph_id));
    let ab = link("L", &a, &b);
    seed(store, &[&a, &b], &[&ab]).await;
    let subgraph = store
        .get_subgraph(graph_id, &[a.id(), EntityId::new(), b.id(), a.id()])
        .await
        .unwrap();
    assert_eq!(ids(subgraph.entities()), sorted(vec![a.id(), b.id()]));
    assert_eq!(subgraph.relationships(), &[ab]);
    assert!(store.get_subgraph(graph_id, &[]).await.unwrap().is_empty());
    cleanup(store, &[graph_id]).await;
}

pub async fn subgraph_is_isolated_per_graph(store: &dyn GraphStore) {
    let (graph_a, graph_b) = (GraphId::new(), GraphId::new());
    let (a, b) = (node(graph_a), node(graph_a));
    let ab = link("L", &a, &b);
    seed(store, &[&a, &b], &[&ab]).await;
    let subgraph = store.get_subgraph(graph_b, &ids([&a, &b])).await.unwrap();
    assert!(subgraph.is_empty());
    assert!(subgraph.relationships().is_empty());
    cleanup(store, &[graph_a]).await;
}

// --- delete & clear --------------------------------------------------------------

pub async fn delete_removes_entity_and_incident_relationships(store: &dyn GraphStore) {
    let graph_id = GraphId::new();
    let (a, b, c) = (node(graph_id), node(graph_id), node(graph_id));
    let ab = link("L", &a, &b);
    let bc = link("L", &b, &c);
    let ac = link("L", &a, &c);
    seed(store, &[&a, &b, &c], &[&ab, &bc, &ac]).await;

    assert!(store.delete_entity(graph_id, b.id()).await.unwrap());
    assert_eq!(store.get_entity(graph_id, b.id()).await.unwrap(), None);
    let remaining = store
        .get_subgraph(graph_id, &ids([&a, &b, &c]))
        .await
        .unwrap();
    assert_eq!(ids(remaining.entities()), sorted(vec![a.id(), c.id()]));
    assert_eq!(
        remaining.relationships(),
        &[ac],
        "other relationships untouched"
    );
    cleanup(store, &[graph_id]).await;
}

pub async fn delete_missing_entity_returns_false(store: &dyn GraphStore) {
    assert!(
        !store
            .delete_entity(GraphId::new(), EntityId::new())
            .await
            .unwrap()
    );
}

pub async fn delete_is_isolated_per_graph(store: &dyn GraphStore) {
    let (graph_a, graph_b) = (GraphId::new(), GraphId::new());
    let a = node(graph_a);
    seed(store, &[&a], &[]).await;
    assert!(
        !store.delete_entity(graph_b, a.id()).await.unwrap(),
        "an entity of graph A cannot be deleted through graph B"
    );
    assert_eq!(store.get_entity(graph_a, a.id()).await.unwrap(), Some(a));
    cleanup(store, &[graph_a]).await;
}

pub async fn clear_graph_leaves_other_graphs_untouched(store: &dyn GraphStore) {
    let (graph_a, graph_b) = (GraphId::new(), GraphId::new());
    let (a1, a2) = (node(graph_a), node(graph_a));
    let a_link = link("L", &a1, &a2);
    let (b1, b2) = (node(graph_b), node(graph_b));
    let b_link = link("L", &b1, &b2);
    // Graph B even reuses one of A's entity ids.
    let b_twin = twin(&a1, graph_b);
    seed(store, &[&a1, &a2], &[&a_link]).await;
    seed(store, &[&b1, &b2, &b_twin], &[&b_link]).await;

    store.clear_graph(graph_a).await.unwrap();

    assert!(
        store
            .search_entities(graph_a, &EntitySearch::new())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.get_entity(graph_a, a1.id()).await.unwrap(), None);
    let b_all = store
        .search_entities(graph_b, &EntitySearch::new())
        .await
        .unwrap();
    assert_eq!(ids(&b_all), sorted(ids([&b1, &b2, &b_twin])));
    let b_sub = store
        .get_subgraph(graph_b, &ids([&b1, &b2, &b_twin]))
        .await
        .unwrap();
    assert_eq!(b_sub.relationships(), &[b_link]);

    store.clear_graph(graph_a).await.unwrap(); // idempotent
    cleanup(store, &[graph_b]).await;
}
