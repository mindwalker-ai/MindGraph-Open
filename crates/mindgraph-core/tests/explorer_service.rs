//! `GraphExplorerService` over the in-memory stores: graph checks, isolation,
//! exact search, neighbours, shortest paths and subgraphs. Offline.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::collections::BTreeMap;
use std::sync::Arc;

use mindgraph_core::{
    ApplicationError, CreateGraphProject, EntitySearchRequest, GraphExplorerService,
    GraphProjectService, MemoryMetadataStore, MetadataStore, OntologyDefinition, OntologyService,
};
use mindgraph_domain::{
    Entity, EntityId, GraphId, Properties, PropertyKey, PropertyValue, Relationship,
    RelationshipId, TypeName,
};
use mindgraph_graph::{Direction, GraphStore, MemoryGraphStore, NeighborQuery, PathQuery};
use serde_json::{Value, json};

struct Fixture {
    explorer: GraphExplorerService,
    graph_id: GraphId,
    other_graph: GraphId,
    customer_a: EntityId,
    customer_b: EntityId,
    account: EntityId,
    device: EntityId,
    foreign: EntityId,
    owns: RelationshipId,
}

fn properties(values: &[(&str, PropertyValue)]) -> Properties {
    let mut properties = Properties::new();
    for (key, value) in values {
        properties
            .insert(PropertyKey::new(*key).unwrap(), value.clone())
            .unwrap();
    }
    properties
}

fn entity(graph_id: GraphId, entity_type: &str, values: &[(&str, PropertyValue)]) -> Entity {
    Entity::new(
        EntityId::new(),
        graph_id,
        TypeName::new(entity_type).unwrap(),
        properties(values),
    )
}

fn relationship(
    graph_id: GraphId,
    relationship_type: &str,
    source: &Entity,
    target: &Entity,
) -> Relationship {
    Relationship::new(
        RelationshipId::new(),
        graph_id,
        TypeName::new(relationship_type).unwrap(),
        source.id(),
        target.id(),
        Properties::new(),
    )
}

fn ontology() -> OntologyDefinition {
    serde_json::from_value(json!({
        "entity_types": [
            {"name": "Customer", "properties": [
                {"name": "customer_id", "type": "string", "required": true},
                {"name": "name", "type": "string"},
                {"name": "age", "type": "integer"}]},
            {"name": "Account", "properties": [
                {"name": "account_id", "type": "string", "required": true}]},
            {"name": "Device", "properties": [
                {"name": "device_id", "type": "string", "required": true}]}
        ],
        "relationship_types": [
            {"name": "OWNS", "source_entity_type": "Customer", "target_entity_type": "Account"},
            {"name": "USES", "source_entity_type": "Customer", "target_entity_type": "Device"}
        ]
    }))
    .unwrap()
}

/// Customer A ─OWNS→ Account, A ─USES→ Device 921 ←USES─ Customer B, plus
/// a customer with the same id in another graph.
async fn fixture() -> Fixture {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let store = Arc::new(MemoryGraphStore::new());
    let graphs = GraphProjectService::bootstrap(Arc::clone(&metadata))
        .await
        .unwrap();
    let ontologies = OntologyService::new(Arc::clone(&metadata), graphs.workspace_id());
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
    let string = |value: &str| PropertyValue::String(value.into());
    let a = entity(
        graph_id,
        "Customer",
        &[
            ("customer_id", string("C001")),
            ("name", string("Alice Tan")),
            ("age", PropertyValue::Integer(9_007_199_254_740_993)),
        ],
    );
    let b = entity(graph_id, "Customer", &[("customer_id", string("C002"))]);
    let account = entity(graph_id, "Account", &[("account_id", string("A1001"))]);
    let device = entity(graph_id, "Device", &[("device_id", string("DEVICE-921"))]);
    let foreign = entity(other_graph, "Customer", &[("customer_id", string("C001"))]);
    let owns = relationship(graph_id, "OWNS", &a, &account);
    store
        .create_entities(vec![a.clone(), b.clone(), account.clone(), device.clone()])
        .await
        .unwrap();
    store.create_entity(foreign.clone()).await.unwrap();
    store
        .create_relationships(vec![
            owns.clone(),
            relationship(graph_id, "USES", &a, &device),
            relationship(graph_id, "USES", &b, &device),
        ])
        .await
        .unwrap();
    Fixture {
        explorer: GraphExplorerService::new(metadata, store, graphs.workspace_id()),
        graph_id,
        other_graph,
        customer_a: a.id(),
        customer_b: b.id(),
        account: account.id(),
        device: device.id(),
        foreign: foreign.id(),
        owns: owns.id(),
    }
}

fn search(entity_type: Option<&str>, filters: Value, limit: Option<usize>) -> EntitySearchRequest {
    EntitySearchRequest {
        entity_type: entity_type.map(str::to_owned),
        property_filters: serde_json::from_value::<BTreeMap<String, Value>>(filters).unwrap(),
        limit,
    }
}

fn sorted(mut ids: Vec<EntityId>) -> Vec<EntityId> {
    ids.sort();
    ids
}

fn ids(entities: &[Entity]) -> Vec<EntityId> {
    sorted(entities.iter().map(Entity::id).collect())
}

#[tokio::test]
async fn entities_are_read_within_their_graph_only() {
    let f = fixture().await;
    let entity = f
        .explorer
        .get_entity(f.graph_id, f.customer_a)
        .await
        .unwrap();
    assert_eq!(entity.entity_type().as_str(), "Customer");
    assert_eq!(
        entity.properties().get("age"),
        Some(&PropertyValue::Integer(9_007_199_254_740_993))
    );

    for (graph, id) in [
        (f.graph_id, EntityId::new()),
        // The other graph's entity is missing here, and vice versa.
        (f.graph_id, f.foreign),
        (f.other_graph, f.customer_a),
    ] {
        assert!(matches!(
            f.explorer.get_entity(graph, id).await,
            Err(ApplicationError::EntityNotFound(missing)) if missing == id
        ));
    }
    assert!(matches!(
        f.explorer.get_entity(GraphId::new(), f.customer_a).await,
        Err(ApplicationError::GraphNotFound(_))
    ));
}

#[tokio::test]
async fn exact_search_by_type_property_and_limit() {
    let f = fixture().await;
    let by_id = f
        .explorer
        .search_entities(
            f.graph_id,
            &search(None, json!({"customer_id": "C001"}), None),
        )
        .await
        .unwrap();
    assert_eq!(
        ids(&by_id),
        [f.customer_a],
        "the other graph's C001 is not found"
    );

    let customers = f
        .explorer
        .search_entities(f.graph_id, &search(Some("Customer"), json!({}), None))
        .await
        .unwrap();
    assert_eq!(customers.len(), 2);
    let limited = f
        .explorer
        .search_entities(f.graph_id, &search(Some("Customer"), json!({}), Some(1)))
        .await
        .unwrap();
    assert_eq!(limited.len(), 1);

    // Big integers match exactly when sent as decimal strings.
    let by_age = f
        .explorer
        .search_entities(
            f.graph_id,
            &search(Some("Customer"), json!({"age": "9007199254740993"}), None),
        )
        .await
        .unwrap();
    assert_eq!(ids(&by_age), [f.customer_a]);
    let none = f
        .explorer
        .search_entities(
            f.graph_id,
            &search(None, json!({"customer_id": "C999"}), None),
        )
        .await
        .unwrap();
    assert!(none.is_empty());

    for bad in [
        search(Some("Customer"), json!({}), Some(0)),
        search(Some("Customer"), json!({}), Some(1001)),
        search(Some("Customer"), json!({"age": "old"}), None),
    ] {
        assert!(matches!(
            f.explorer.search_entities(f.graph_id, &bad).await,
            Err(ApplicationError::InvalidRequest(_))
        ));
    }
}

#[tokio::test]
async fn neighbours_follow_direction_and_relationship_type() {
    let f = fixture().await;
    let both = f
        .explorer
        .neighbors(f.graph_id, f.customer_a, &NeighborQuery::default())
        .await
        .unwrap();
    assert_eq!(
        ids(both.entities()),
        sorted(vec![f.customer_a, f.account, f.device])
    );
    assert_eq!(both.relationships().len(), 2);

    let incoming = f
        .explorer
        .neighbors(
            f.graph_id,
            f.device,
            &NeighborQuery::new(Direction::Incoming),
        )
        .await
        .unwrap();
    assert_eq!(
        incoming.relationships().len(),
        2,
        "two customers use the device"
    );
    let outgoing = f
        .explorer
        .neighbors(
            f.graph_id,
            f.device,
            &NeighborQuery::new(Direction::Outgoing),
        )
        .await
        .unwrap();
    assert!(outgoing.relationships().is_empty());

    let owns_only = f
        .explorer
        .neighbors(
            f.graph_id,
            f.customer_a,
            &NeighborQuery::new(Direction::Both)
                .with_relationship_type(TypeName::new("OWNS").unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(
        owns_only
            .relationships()
            .iter()
            .map(Relationship::id)
            .collect::<Vec<_>>(),
        [f.owns]
    );
    let limited = f
        .explorer
        .neighbors(
            f.graph_id,
            f.customer_a,
            &NeighborQuery::new(Direction::Both).with_limit(1).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(limited.relationships().len(), 1);

    assert!(matches!(
        f.explorer
            .neighbors(f.graph_id, f.foreign, &NeighborQuery::default())
            .await,
        Err(ApplicationError::EntityNotFound(_))
    ));
}

#[tokio::test]
async fn shortest_paths_are_bounded_by_depth_and_direction() {
    let f = fixture().await;
    let path = f
        .explorer
        .find_path(
            f.graph_id,
            f.customer_a,
            f.customer_b,
            &PathQuery::default(),
        )
        .await
        .unwrap()
        .expect("A and B share device 921");
    assert_eq!(path.hops(), 2);
    assert_eq!(
        path.entities().iter().map(Entity::id).collect::<Vec<_>>(),
        [f.customer_a, f.device, f.customer_b]
    );

    // B is reached only against the second USES edge.
    let outgoing = f
        .explorer
        .find_path(
            f.graph_id,
            f.customer_a,
            f.customer_b,
            &PathQuery::new(Direction::Outgoing),
        )
        .await
        .unwrap();
    assert!(outgoing.is_none());
    let too_short = f
        .explorer
        .find_path(
            f.graph_id,
            f.customer_a,
            f.customer_b,
            &PathQuery::new(Direction::Both).with_max_depth(1).unwrap(),
        )
        .await
        .unwrap();
    assert!(too_short.is_none(), "no path within one hop");

    let itself = f
        .explorer
        .find_path(f.graph_id, f.account, f.account, &PathQuery::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(itself.hops(), 0);
    assert_eq!(itself.entities().len(), 1);

    assert!(matches!(
        f.explorer
            .find_path(f.graph_id, f.customer_a, f.foreign, &PathQuery::default())
            .await,
        Err(ApplicationError::EntityNotFound(id)) if id == f.foreign
    ));
}

#[tokio::test]
async fn subgraphs_are_bounded_and_scoped() {
    let f = fixture().await;
    let subgraph = f
        .explorer
        .subgraph(
            f.graph_id,
            &[f.customer_a, f.account, f.foreign, f.customer_a],
        )
        .await
        .unwrap();
    assert_eq!(
        ids(subgraph.entities()),
        sorted(vec![f.customer_a, f.account])
    );
    assert_eq!(
        subgraph
            .relationships()
            .iter()
            .map(Relationship::id)
            .collect::<Vec<_>>(),
        [f.owns]
    );

    let empty = f.explorer.subgraph(f.graph_id, &[]).await.unwrap();
    assert!(empty.entities().is_empty() && empty.relationships().is_empty());

    let too_many: Vec<EntityId> = (0..1001).map(|_| EntityId::new()).collect();
    assert!(matches!(
        f.explorer.subgraph(f.graph_id, &too_many).await,
        Err(ApplicationError::InvalidRequest(_))
    ));
    assert!(matches!(
        f.explorer.subgraph(GraphId::new(), &[f.customer_a]).await,
        Err(ApplicationError::GraphNotFound(_))
    ));
}
