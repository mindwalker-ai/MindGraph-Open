//! Deterministic GraphRAG retrieval over the in-memory GraphStore: no LLM,
//! no Docker. Intents are written as the AI service would return them, then
//! validated and executed exactly as in production.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Panicking is the intended failure mode in tests.

use std::collections::HashSet;

use mindgraph_domain::{
    Entity, EntityId, EntityType, EntityTypeId, GraphId, Ontology, OntologyVersion, Properties,
    PropertiesSchema, PropertyDefinition, PropertyKey, PropertyType, Relationship, RelationshipId,
    RelationshipType, RelationshipTypeId, TypeName,
};
use mindgraph_graph::{GraphStore, MemoryGraphStore};
use mindgraph_graphrag::{
    ContextMeta, GraphRagLimits, PlanContext, PlanError, Resolution, RetrievalIntent,
    RetrievalOutcome, ValidatedPlan, build_context, retrieve, validate_intent,
};
use serde_json::{Value, json};

struct Fixture {
    store: MemoryGraphStore,
    graph_id: GraphId,
    ontology: Ontology,
    customers: Vec<Entity>,
    devices: Vec<Entity>,
    uses: Vec<Relationship>,
}

fn props(values: &[(&str, &str)]) -> Properties {
    let mut properties = Properties::new();
    for (key, value) in values {
        properties
            .insert(PropertyKey::new(*key).unwrap(), *value)
            .unwrap();
    }
    properties
}

fn entity(graph_id: GraphId, entity_type: &str, values: &[(&str, &str)]) -> Entity {
    Entity::new(
        EntityId::new(),
        graph_id,
        TypeName::new(entity_type).unwrap(),
        props(values),
    )
}

fn link(kind: &str, source: &Entity, target: &Entity) -> Relationship {
    Relationship::new(
        RelationshipId::new(),
        source.graph_id(),
        TypeName::new(kind).unwrap(),
        source.id(),
        target.id(),
        Properties::new(),
    )
}

fn ontology(graph_id: GraphId) -> Ontology {
    let property = |name: &str| PropertyDefinition {
        key: PropertyKey::new(name).unwrap(),
        property_type: PropertyType::String,
        required: false,
    };
    let entity = |name: &str, properties: &[&str]| {
        EntityType::new(
            EntityTypeId::new(),
            graph_id,
            TypeName::new(name).unwrap(),
            PropertiesSchema::new(properties.iter().map(|p| property(p)).collect()).unwrap(),
        )
    };
    let relationship = |name: &str, source: &str, target: &str| {
        RelationshipType::new(
            RelationshipTypeId::new(),
            graph_id,
            TypeName::new(name).unwrap(),
            TypeName::new(source).unwrap(),
            TypeName::new(target).unwrap(),
            PropertiesSchema::default(),
        )
    };
    Ontology::new(
        graph_id,
        OntologyVersion::INITIAL,
        vec![
            entity("Customer", &["customer_id", "name", "city"]),
            entity("Account", &["account_id"]),
            entity("Device", &["device_id"]),
        ],
        vec![
            relationship("OWNS", "Customer", "Account"),
            relationship("USES", "Customer", "Device"),
            relationship("REFERS", "Customer", "Customer"),
        ],
    )
    .unwrap()
}

/// Alice (C001) and Budi (C002) share DEVICE-921 (Alice twice: parallel
/// relationships); Alice also uses DEVICE-109; Dewi (C004) uses DEVICE-447;
/// Citra (C003) refers herself (a self-loop). Alice owns A1001. Another
/// graph holds a second C001 and a second DEVICE-921.
async fn fixture() -> Fixture {
    let store = MemoryGraphStore::new();
    let graph_id = GraphId::new();
    let customer = |id: &str, name: &str, city: &str| {
        entity(
            graph_id,
            "Customer",
            &[("customer_id", id), ("name", name), ("city", city)],
        )
    };
    let customers = vec![
        customer("C001", "Alice Tan", "Jakarta"),
        customer("C002", "Budi Santoso", "Bandung"),
        customer("C003", "Citra Lestari", "Surabaya"),
        customer("C004", "Dewi Kusuma", "Jakarta"),
    ];
    let devices = vec![
        entity(graph_id, "Device", &[("device_id", "DEVICE-921")]),
        entity(graph_id, "Device", &[("device_id", "DEVICE-109")]),
        entity(graph_id, "Device", &[("device_id", "DEVICE-447")]),
    ];
    let account = entity(graph_id, "Account", &[("account_id", "A1001")]);
    let uses = vec![
        link("USES", &customers[0], &devices[0]),
        link("USES", &customers[0], &devices[0]),
        link("USES", &customers[1], &devices[0]),
        link("USES", &customers[0], &devices[1]),
        link("USES", &customers[3], &devices[2]),
    ];
    let mut all: Vec<Entity> = customers.clone();
    all.extend(devices.clone());
    all.push(account.clone());
    store.create_entities(all).await.unwrap();
    let mut relationships = uses.clone();
    relationships.push(link("OWNS", &customers[0], &account));
    relationships.push(link("REFERS", &customers[2], &customers[2]));
    store.create_relationships(relationships).await.unwrap();

    // Another graph with look-alike data.
    let other = GraphId::new();
    let twin = entity(
        other,
        "Customer",
        &[("customer_id", "C001"), ("name", "Alice Tan")],
    );
    let twin_device = entity(other, "Device", &[("device_id", "DEVICE-921")]);
    let twin_use = link("USES", &twin, &twin_device);
    store
        .create_entities(vec![twin, twin_device])
        .await
        .unwrap();
    store.create_relationship(twin_use).await.unwrap();

    Fixture {
        store,
        graph_id,
        ontology: ontology(graph_id),
        customers,
        devices,
        uses,
    }
}

impl Fixture {
    fn plan(
        &self,
        intent: Value,
        selected: &[EntityId],
        limits: &GraphRagLimits,
    ) -> Result<ValidatedPlan, PlanError> {
        let intent: RetrievalIntent = serde_json::from_value(intent).unwrap();
        validate_intent(
            &intent,
            &PlanContext {
                ontology: &self.ontology,
                selected,
                earlier: &[],
                limits,
            },
        )
    }

    async fn ask(&self, intent: Value, selected: &[EntityId]) -> RetrievalOutcome {
        self.ask_with(intent, selected, &GraphRagLimits::default())
            .await
    }

    async fn ask_with(
        &self,
        intent: Value,
        selected: &[EntityId],
        limits: &GraphRagLimits,
    ) -> RetrievalOutcome {
        let plan = self.plan(intent, selected, limits).unwrap();
        let outcome = retrieve(&self.store, self.graph_id, &plan.plan, limits)
            .await
            .unwrap();
        outcome.evidence.validate(self.graph_id).unwrap();
        outcome
    }
}

fn ids(entities: &[Entity]) -> HashSet<EntityId> {
    entities.iter().map(Entity::id).collect()
}

#[tokio::test]
async fn exact_entity_lookup() {
    let f = fixture().await;
    let outcome = f
        .ask(
            json!({"operation": "entity_lookup",
                   "source": {"entity_type": "Customer", "match": {"customer_id": "C002"}}}),
            &[],
        )
        .await;
    assert_eq!(outcome.resolution, Resolution::Resolved);
    assert_eq!(
        ids(outcome.evidence.entities()),
        HashSet::from([f.customers[1].id()])
    );
    assert!(!outcome.stats.truncated);

    // Exact only: no fuzzy or case-insensitive matching.
    let outcome = f
        .ask(
            json!({"operation": "entity_lookup",
                   "source": {"entity_type": "Customer", "match": {"customer_id": "c002"}}}),
            &[],
        )
        .await;
    assert_eq!(outcome.resolution, Resolution::NotFound);
    assert!(outcome.evidence.is_empty());
}

#[tokio::test]
async fn neighbors_keep_parallel_relationships_and_direction() {
    let f = fixture().await;
    let outcome = f
        .ask(
            json!({"operation": "neighbors", "source": {"ref": "S1"}, "relationship_type": "USES"}),
            &[f.customers[0].id()],
        )
        .await;
    assert_eq!(
        ids(outcome.evidence.entities()),
        HashSet::from([f.customers[0].id(), f.devices[0].id(), f.devices[1].id()])
    );
    assert_eq!(
        outcome.evidence.relationships().len(),
        3,
        "both parallel USES to DEVICE-921"
    );
    for relationship in outcome.evidence.relationships() {
        assert_eq!(
            relationship.source_id(),
            f.customers[0].id(),
            "direction preserved"
        );
    }
}

#[tokio::test]
async fn two_hop_neighbors_with_a_type_filter() {
    let f = fixture().await;
    let outcome = f
        .ask(
            json!({"operation": "neighbors", "source": {"ref": "S1"}, "entity_type": "Customer",
                   "relationship_type": "USES", "max_depth": 2}),
            &[f.customers[0].id()],
        )
        .await;
    // Alice → DEVICE-921 → Budi; DEVICE-109 leads to nobody else.
    assert_eq!(
        ids(outcome.evidence.entities()),
        HashSet::from([f.customers[0].id(), f.devices[0].id(), f.customers[1].id()])
    );
    assert_eq!(
        outcome.stats.operations, 4,
        "resolve the centre, expand it, expand its two devices"
    );
}

#[tokio::test]
async fn shortest_path_is_ordered_and_bounded() {
    let f = fixture().await;
    let selected = [f.customers[0].id(), f.customers[1].id()];
    let outcome = f
        .ask(
            json!({"operation": "find_path", "source": {"ref": "S1"}, "target": {"ref": "S2"}}),
            &selected,
        )
        .await;
    let path = &outcome.evidence.paths()[0];
    assert_eq!(
        path.entity_ids(),
        [f.customers[0].id(), f.devices[0].id(), f.customers[1].id()]
    );
    assert_eq!(path.relationship_ids().len(), 2);

    // Dewi is not connected to Alice: a bounded failure, not a proof.
    let outcome = f
        .ask(
            json!({"operation": "find_path", "source": {"ref": "S1"},
                   "target": {"entity_type": "Customer", "match": {"customer_id": "C004"}},
                   "max_depth": 4}),
            &[f.customers[0].id()],
        )
        .await;
    assert_eq!(outcome.resolution, Resolution::NoPath);
    assert!(outcome.evidence.paths().is_empty());
    assert!(
        outcome
            .limitations
            .iter()
            .any(|l| l.contains("within 4 hops") && l.contains("may still exist"))
    );
}

#[tokio::test]
async fn shared_neighbor_from_a_selected_customer() {
    let f = fixture().await;
    let outcome = f
        .ask(
            json!({"operation": "shared_neighbor", "source": {"ref": "S1"}, "entity_type": "Customer",
                   "shared_entity_type": "Device", "relationship_type": "USES"}),
            &[f.customers[0].id()],
        )
        .await;
    assert_eq!(outcome.resolution, Resolution::Resolved);
    let groups = outcome.evidence.groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].shared, f.devices[0].id());
    let mut members = vec![f.customers[0].id(), f.customers[1].id()];
    members.sort();
    assert_eq!(groups[0].members, members);
    assert_eq!(
        ids(outcome.evidence.entities()),
        HashSet::from([f.customers[0].id(), f.customers[1].id(), f.devices[0].id()])
    );
    // Every USES to DEVICE-921 (including the parallel one), nothing else.
    let expected: HashSet<RelationshipId> = f.uses[..3].iter().map(Relationship::id).collect();
    let found: HashSet<RelationshipId> = outcome
        .evidence
        .relationships()
        .iter()
        .map(Relationship::id)
        .collect();
    assert_eq!(found, expected);
    assert_eq!(outcome.evidence.paths().len(), 1);
    assert_eq!(
        outcome.stats.candidates_inspected, 2,
        "DEVICE-921 and DEVICE-109"
    );
    assert!(!outcome.stats.truncated);
}

#[tokio::test]
async fn shared_neighbor_from_the_shared_entity_stays_in_its_graph() {
    let f = fixture().await;
    let outcome = f
        .ask(
            json!({"operation": "shared_neighbor",
                   "source": {"entity_type": "Device", "match": {"device_id": "DEVICE-921"}},
                   "entity_type": "Customer", "shared_entity_type": "Device"}),
            &[],
        )
        .await;
    assert_eq!(outcome.evidence.groups().len(), 1);
    let own: HashSet<EntityId> = f
        .customers
        .iter()
        .chain(&f.devices)
        .map(Entity::id)
        .collect();
    assert!(
        ids(outcome.evidence.entities()).is_subset(&own),
        "the other graph's look-alike C001 and DEVICE-921 never appear"
    );
}

#[tokio::test]
async fn bounded_discovery_reports_truncation() {
    let f = fixture().await;
    let intent = json!({"operation": "shared_neighbor", "entity_type": "Customer",
                        "shared_entity_type": "Device"});
    let complete = f.ask(intent.clone(), &[]).await;
    assert_eq!(complete.evidence.groups().len(), 1);
    assert_eq!(complete.stats.candidates_inspected, 3);
    assert!(
        !complete.stats.truncated,
        "all three devices were inspected"
    );

    let limits = GraphRagLimits {
        max_candidates: 2,
        ..GraphRagLimits::default()
    };
    let bounded = f.ask_with(intent.clone(), &[], &limits).await;
    assert!(bounded.stats.truncated);
    assert_eq!(bounded.stats.candidates_inspected, 2);
    assert!(
        bounded
            .limitations
            .iter()
            .any(|l| l.contains("first 2 Device entities"))
    );

    let tight = GraphRagLimits {
        max_operations: 2,
        ..GraphRagLimits::default()
    };
    let stopped = f.ask_with(intent, &[], &tight).await;
    assert!(stopped.stats.truncated);
    assert_eq!(stopped.stats.operations, 2);
}

#[tokio::test]
async fn the_evidence_budget_truncates_without_dangling_relationships() {
    let f = fixture().await;
    let limits = GraphRagLimits {
        max_evidence_entities: 2,
        ..GraphRagLimits::default()
    };
    let outcome = f
        .ask_with(
            json!({"operation": "neighbors", "source": {"ref": "S1"}}),
            &[f.customers[0].id()],
            &limits,
        )
        .await;
    assert!(outcome.stats.truncated);
    assert_eq!(outcome.evidence.entities().len(), 2);
    assert!(
        outcome
            .limitations
            .iter()
            .any(|l| l.contains("evidence budget"))
    );
}

#[tokio::test]
async fn self_loops_are_kept_once() {
    let f = fixture().await;
    let outcome = f
        .ask(
            json!({"operation": "neighbors",
                   "source": {"entity_type": "Customer", "match": {"customer_id": "C003"}}}),
            &[],
        )
        .await;
    assert_eq!(outcome.evidence.entities().len(), 1);
    let loops = outcome.evidence.relationships();
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0].source_id(), loops[0].target_id());
}

#[tokio::test]
async fn missing_and_ambiguous_anchors_are_reported() {
    let f = fixture().await;
    let missing = f
        .ask(
            json!({"operation": "neighbors", "source": {"entity_id": EntityId::new().to_string()}}),
            &[],
        )
        .await;
    assert_eq!(missing.resolution, Resolution::NotFound);
    assert!(missing.evidence.is_empty());

    // Two customers live in Jakarta: a neighbourhood centre must be unique.
    let ambiguous = f
        .ask(
            json!({"operation": "neighbors",
                   "source": {"entity_type": "Customer", "match": {"city": "Jakarta"}}}),
            &[],
        )
        .await;
    assert_eq!(ambiguous.resolution, Resolution::Ambiguous);
    assert_eq!(
        ids(ambiguous.evidence.entities()),
        HashSet::from([f.customers[0].id(), f.customers[3].id()]),
        "the candidates are shown, none is chosen"
    );
    assert!(ambiguous.evidence.relationships().is_empty());
}

#[tokio::test]
async fn invalid_plans_never_reach_the_store() {
    let f = fixture().await;
    let limits = GraphRagLimits::default();
    for intent in [
        json!({"operation": "neighbors", "source": {"entity_type": "Merchant"}}),
        json!({"operation": "shared_neighbor", "entity_type": "Customer",
               "shared_entity_type": "Device", "relationship_type": "OWNS"}),
        json!({"operation": "neighbors", "source": {"ref": "S1"}}),
    ] {
        assert!(matches!(
            f.plan(intent, &[], &limits),
            Err(PlanError::Invalid(_))
        ));
    }
}

#[tokio::test]
async fn retrieval_and_context_are_deterministic() {
    let f = fixture().await;
    let intent = json!({"operation": "shared_neighbor", "entity_type": "Customer",
                        "shared_entity_type": "Device"});
    let first = f.ask(intent.clone(), &[]).await;
    let second = f.ask(intent, &[]).await;
    assert_eq!(first, second);
    let meta = ContextMeta {
        scope: "bounded_graph",
        operation: mindgraph_graphrag::Operation::SharedNeighbor,
        complete: true,
        candidates_inspected: first.stats.candidates_inspected,
        limitations: &first.limitations,
    };
    let a = build_context(&first.evidence, &meta, &GraphRagLimits::default());
    let b = build_context(&second.evidence, &meta, &GraphRagLimits::default());
    assert_eq!(
        serde_json::to_string(&a.value).unwrap(),
        serde_json::to_string(&b.value).unwrap()
    );
    assert_eq!(
        a.value["shared_groups"][0]["members"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
