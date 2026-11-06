//! `EntityResolutionService` over the in-memory stores (M9): bounded scans,
//! review decisions, canonical identity and, above all, proof that logical
//! identity approval never writes to the GraphStore. Offline.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)] // Panicking is the intended failure mode in tests.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use mindgraph_core::matching::{RuleSpec, SignalNote, SignalOutcome};
use mindgraph_core::{
    ApplicationError, CandidateQuery, CandidateStatus, CreateGraphProject, DecisionKind,
    DecisionRequest, EntityResolutionConfig, EntityResolutionService, EntityResolutionStore,
    GraphExplorerService, GraphProjectService, MemoryEntityResolutionStore, MemoryMetadataStore,
    MetadataStore, NewCandidate, OntologyDefinition, OntologyService, ResolutionError,
    ResolutionScan, ScanRequest, SkippedPairs,
};
use mindgraph_domain::{
    Entity, EntityId, GraphId, OntologyVersion, Properties, PropertyKey, PropertyValue,
    Relationship, RelationshipId, ResolutionCandidateId, ResolutionScanId, TypeName,
};
use mindgraph_graph::{
    EntitySearch, GraphPath, GraphStore, GraphStoreError, MemoryGraphStore, NeighborQuery,
    PathQuery, Subgraph,
};
use serde_json::json;

/// A GraphStore that counts writes and can slow down searches.
struct Spy {
    inner: MemoryGraphStore,
    writes: AtomicUsize,
    search_delay: Duration,
}

impl Spy {
    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }

    fn write(&self) {
        self.writes.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl GraphStore for Spy {
    async fn create_entity(&self, entity: Entity) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.create_entity(entity).await
    }
    async fn create_entities(&self, entities: Vec<Entity>) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.create_entities(entities).await
    }
    async fn create_relationship(&self, relationship: Relationship) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.create_relationship(relationship).await
    }
    async fn create_relationships(
        &self,
        relationships: Vec<Relationship>,
    ) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.create_relationships(relationships).await
    }
    async fn get_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<Option<Entity>, GraphStoreError> {
        self.inner.get_entity(graph_id, entity_id).await
    }
    async fn search_entities(
        &self,
        graph_id: GraphId,
        search: &EntitySearch,
    ) -> Result<Vec<Entity>, GraphStoreError> {
        if !self.search_delay.is_zero() {
            tokio::time::sleep(self.search_delay).await;
        }
        self.inner.search_entities(graph_id, search).await
    }
    async fn get_neighbors(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
        query: &NeighborQuery,
    ) -> Result<Subgraph, GraphStoreError> {
        self.inner.get_neighbors(graph_id, entity_id, query).await
    }
    async fn find_path(
        &self,
        graph_id: GraphId,
        source: EntityId,
        target: EntityId,
        query: &PathQuery,
    ) -> Result<Option<GraphPath>, GraphStoreError> {
        self.inner.find_path(graph_id, source, target, query).await
    }
    async fn get_subgraph(
        &self,
        graph_id: GraphId,
        entity_ids: &[EntityId],
    ) -> Result<Subgraph, GraphStoreError> {
        self.inner.get_subgraph(graph_id, entity_ids).await
    }
    async fn delete_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<bool, GraphStoreError> {
        self.write();
        self.inner.delete_entity(graph_id, entity_id).await
    }
    async fn clear_graph(&self, graph_id: GraphId) -> Result<(), GraphStoreError> {
        self.write();
        self.inner.clear_graph(graph_id).await
    }
}

struct Fixture {
    service: EntityResolutionService,
    store: Arc<MemoryEntityResolutionStore>,
    graph: Arc<Spy>,
    graph_id: GraphId,
    other_graph: GraphId,
    ikhwan_a: EntityId,
    ikhwan_b: EntityId,
    dewi_a: EntityId,
    dewi_b: EntityId,
    merchant: EntityId,
    /// Writes done by the fixture itself.
    setup_writes: usize,
}

fn ontology() -> OntologyDefinition {
    serde_json::from_value(json!({
        "entity_types": [
            {"name": "Customer", "properties": [
                {"name": "customer_id", "type": "string", "required": true},
                {"name": "name", "type": "string"},
                {"name": "email", "type": "string"},
                {"name": "phone", "type": "string"},
                {"name": "city", "type": "string"},
                {"name": "age", "type": "integer"}]},
            {"name": "Merchant", "properties": [{"name": "name", "type": "string"}]},
            {"name": "Account", "properties": [
                {"name": "account_id", "type": "string", "required": true}]}
        ],
        "relationship_types": [
            {"name": "OWNS", "source_entity_type": "Customer", "target_entity_type": "Account"}
        ]
    }))
    .unwrap()
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
    Relationship::new(
        RelationshipId::new(),
        graph_id,
        TypeName::new("OWNS").unwrap(),
        customer.id(),
        account.id(),
        Properties::new(),
    )
}

async fn fixture_with(config: EntityResolutionConfig, search_delay: Duration) -> Fixture {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
    let graph = Arc::new(Spy {
        inner: MemoryGraphStore::new(),
        writes: AtomicUsize::new(0),
        search_delay,
    });
    let graphs = GraphProjectService::bootstrap(Arc::clone(&metadata))
        .await
        .unwrap();
    let ontologies = OntologyService::new(Arc::clone(&metadata), graphs.workspace_id());
    let mut ids = Vec::new();
    for name in ["Fraud", "Other"] {
        let project = graphs
            .create_graph(CreateGraphProject {
                name: name.into(),
                description: None,
            })
            .await
            .unwrap();
        ontologies
            .create_version(project.id(), &ontology())
            .await
            .unwrap();
        ids.push(project.id());
    }
    let (graph_id, other_graph) = (ids[0], ids[1]);
    let ikhwan_a = entity(
        graph_id,
        "Customer",
        &[
            ("customer_id", "K-1"),
            ("name", "Muhammad Ikhwan Nasution"),
            ("email", "ikhwan@example.com"),
            ("phone", "+6281212345678"),
        ],
    );
    let ikhwan_b = entity(
        graph_id,
        "Customer",
        &[
            ("customer_id", "K-2"),
            ("name", "M. Ikhwan Nasution"),
            ("email", "IKHWAN@example.com"),
            ("phone", "081212345678"),
        ],
    );
    let dewi_a = entity(
        graph_id,
        "Customer",
        &[
            ("customer_id", "K-3"),
            ("name", "Dewi Kusuma"),
            ("email", "dewi.k@example.org"),
            ("city", "Jakarta"),
        ],
    );
    let dewi_b = entity(
        graph_id,
        "Customer",
        &[
            ("customer_id", "K-4"),
            ("name", "Dewi Kusuma"),
            ("email", "dkusuma@example.net"),
            ("city", "Medan"),
        ],
    );
    let alice = entity(
        graph_id,
        "Customer",
        &[("customer_id", "K-5"), ("name", "Alice Tan")],
    );
    let merchant = entity(graph_id, "Merchant", &[("name", "Ikhwan Nasution")]);
    let account_a = entity(graph_id, "Account", &[("account_id", "A-1")]);
    let account_b = entity(graph_id, "Account", &[("account_id", "A-2")]);
    // The same people in another graph.
    let foreign_a = entity(
        other_graph,
        "Customer",
        &[("customer_id", "K-1"), ("name", "Muhammad Ikhwan Nasution")],
    );
    let foreign_b = entity(
        other_graph,
        "Customer",
        &[("customer_id", "K-2"), ("name", "M. Ikhwan Nasution")],
    );
    graph
        .create_entities(vec![
            ikhwan_a.clone(),
            ikhwan_b.clone(),
            dewi_a.clone(),
            dewi_b.clone(),
            alice,
            merchant.clone(),
            account_a.clone(),
            account_b.clone(),
        ])
        .await
        .unwrap();
    graph
        .create_entities(vec![foreign_a, foreign_b])
        .await
        .unwrap();
    graph
        .create_relationships(vec![
            owns(graph_id, &ikhwan_a, &account_a),
            owns(graph_id, &ikhwan_b, &account_b),
        ])
        .await
        .unwrap();
    let setup_writes = graph.writes();
    let store = Arc::new(MemoryEntityResolutionStore::new());
    let explorer = GraphExplorerService::new(
        metadata,
        Arc::clone(&graph) as Arc<dyn GraphStore>,
        graphs.workspace_id(),
    );
    Fixture {
        service: EntityResolutionService::new(
            explorer,
            Arc::clone(&store) as Arc<dyn EntityResolutionStore>,
            config,
        ),
        store,
        graph,
        graph_id,
        other_graph,
        ikhwan_a: ikhwan_a.id(),
        ikhwan_b: ikhwan_b.id(),
        dewi_a: dewi_a.id(),
        dewi_b: dewi_b.id(),
        merchant: merchant.id(),
        setup_writes,
    }
}

async fn fixture() -> Fixture {
    fixture_with(EntityResolutionConfig::default(), Duration::ZERO).await
}

fn rule(property: &str, kind: &str) -> RuleSpec {
    RuleSpec {
        property: property.into(),
        kind: kind.into(),
        ..RuleSpec::default()
    }
}

fn customer_scan() -> ScanRequest {
    ScanRequest {
        entity_type: "Customer".into(),
        max_entities: None,
        rules: vec![
            rule("email", "normalized_email"),
            RuleSpec {
                country: Some("ID".into()),
                ..rule("phone", "normalized_phone")
            },
            rule("name", "fuzzy_name"),
        ],
    }
}

fn query() -> CandidateQuery {
    CandidateQuery {
        status: None,
        scan_id: None,
        entity_id: None,
        limit: 20,
        after: None,
    }
}

fn same(canonical: EntityId) -> DecisionRequest {
    DecisionRequest {
        kind: DecisionKind::SameEntity,
        canonical_entity_id: Some(canonical),
        reconsider: false,
    }
}

const DIFFERENT: DecisionRequest = DecisionRequest {
    kind: DecisionKind::DifferentEntities,
    canonical_entity_id: None,
    reconsider: false,
};

/// Every entity and relationship of a graph, for before/after comparison.
async fn snapshot(graph: &Spy, graph_id: GraphId) -> (Vec<Entity>, Vec<Relationship>) {
    let entities = graph
        .search_entities(graph_id, &EntitySearch::new().with_limit(1_000).unwrap())
        .await
        .unwrap();
    let ids: Vec<EntityId> = entities.iter().map(Entity::id).collect();
    let subgraph = graph.get_subgraph(graph_id, &ids).await.unwrap();
    (entities, subgraph.relationships().to_vec())
}

async fn candidate_for(f: &Fixture, a: EntityId, b: EntityId) -> ResolutionCandidateId {
    let page = f
        .service
        .list_candidates(f.graph_id, query())
        .await
        .unwrap();
    page.candidates
        .iter()
        .find(|c| BTreeSet::from([c.entity_a, c.entity_b]) == BTreeSet::from([a, b]))
        .map(|c| c.id)
        .expect("candidate exists")
}

fn resolution_error(result: Result<impl std::fmt::Debug, ApplicationError>) -> ResolutionError {
    match result {
        Err(ApplicationError::Resolution(error)) => error,
        other => panic!("expected a resolution error, got {other:?}"),
    }
}

// --- scans -----------------------------------------------------------------------------

#[tokio::test]
async fn a_scan_finds_candidates_with_evidence_and_honest_coverage() {
    let f = fixture().await;
    let scan = f.service.scan(f.graph_id, customer_scan()).await.unwrap();
    assert_eq!(scan.entity_type.as_str(), "Customer");
    assert_eq!(
        scan.entities_examined, 5,
        "the merchant and accounts are not examined"
    );
    assert!(!scan.truncated);
    assert!(scan.exhaustive());
    assert_eq!(scan.candidates_found, 2);
    assert_eq!(scan.ontology_version, OntologyVersion::new(2).unwrap());

    let page = f
        .service
        .list_candidates(f.graph_id, query())
        .await
        .unwrap();
    let ikhwan = &page.candidates[0];
    assert_eq!(
        BTreeSet::from([ikhwan.entity_a, ikhwan.entity_b]),
        BTreeSet::from([f.ikhwan_a, f.ikhwan_b]),
        "the strongest candidate comes first"
    );
    let signal = |property: &str| {
        ikhwan
            .signals
            .iter()
            .find(|s| s.property == property)
            .unwrap()
    };
    assert_eq!(signal("email").outcome, SignalOutcome::Normalized);
    assert_eq!(
        signal("email").note,
        Some(SignalNote::CaseInsensitiveLocalPart)
    );
    assert_eq!(signal("phone").outcome, SignalOutcome::Normalized);
    assert_eq!(signal("name").outcome, SignalOutcome::Fuzzy);
    let dewi = &page.candidates[1];
    assert_eq!(
        BTreeSet::from([dewi.entity_a, dewi.entity_b]),
        BTreeSet::from([f.dewi_a, f.dewi_b])
    );
    assert!(dewi.score < ikhwan.score, "a shared name alone ranks lower");
    assert!(
        page.candidates
            .iter()
            .all(|c| c.status == CandidateStatus::Pending)
    );
    assert!(
        page.candidates
            .iter()
            .all(|c| c.entity_a != f.merchant && c.entity_b != f.merchant)
    );
    assert_eq!(
        f.graph.writes(),
        f.setup_writes,
        "scans never write to the graph"
    );
}

#[tokio::test]
async fn a_bounded_scan_reports_truncation() {
    let f = fixture().await;
    let scan = f
        .service
        .scan(
            f.graph_id,
            ScanRequest {
                max_entities: Some(2),
                ..customer_scan()
            },
        )
        .await
        .unwrap();
    assert_eq!(scan.entities_examined, 2);
    assert!(scan.truncated, "more customers exist than were examined");
    assert!(!scan.exhaustive());
}

#[tokio::test]
async fn invalid_scans_are_rejected() {
    let f = fixture().await;
    let unknown_type = f
        .service
        .scan(
            f.graph_id,
            ScanRequest {
                entity_type: "Planet".into(),
                ..customer_scan()
            },
        )
        .await;
    assert!(matches!(
        resolution_error(unknown_type),
        ResolutionError::UnknownEntityType(_)
    ));
    for rules in [
        vec![rule("unknown", "fuzzy_name")],
        vec![rule("age", "fuzzy_name")],
        vec![RuleSpec {
            threshold: Some(1.5),
            ..rule("name", "fuzzy_name")
        }],
        vec![],
    ] {
        let result = f
            .service
            .scan(
                f.graph_id,
                ScanRequest {
                    rules,
                    ..customer_scan()
                },
            )
            .await;
        assert!(matches!(
            resolution_error(result),
            ResolutionError::InvalidRule(_)
        ));
    }
    for max_entities in [0, 1, 1_000] {
        let result = f
            .service
            .scan(
                f.graph_id,
                ScanRequest {
                    max_entities: Some(max_entities),
                    ..customer_scan()
                },
            )
            .await;
        assert!(
            matches!(result, Err(ApplicationError::InvalidRequest(_))),
            "{max_entities}"
        );
    }
    let missing = f.service.scan(GraphId::new(), customer_scan()).await;
    assert!(matches!(missing, Err(ApplicationError::GraphNotFound(_))));
}

#[tokio::test]
async fn a_slow_scan_times_out() {
    let f = fixture_with(
        EntityResolutionConfig {
            scan_timeout: Duration::from_secs(1),
            ..EntityResolutionConfig::default()
        },
        Duration::from_millis(1_500),
    )
    .await;
    let result = f.service.scan(f.graph_id, customer_scan()).await;
    assert!(matches!(
        resolution_error(result),
        ResolutionError::ScanTimeout(1)
    ));
    assert!(
        f.store.list_scans(f.graph_id, 10).await.unwrap().is_empty(),
        "nothing was stored"
    );
}

// --- decisions and identity ---------------------------------------------------------------

#[tokio::test]
async fn approving_links_identities_without_touching_the_graph() {
    let f = fixture().await;
    let before = snapshot(&f.graph, f.graph_id).await;
    let other_before = snapshot(&f.graph, f.other_graph).await;
    f.service.scan(f.graph_id, customer_scan()).await.unwrap();
    let candidate = candidate_for(&f, f.ikhwan_a, f.ikhwan_b).await;

    // The reviewer chooses which original entity stays canonical.
    let result = f
        .service
        .decide(f.graph_id, candidate, same(f.ikhwan_b))
        .await
        .unwrap();
    assert!(!result.outcome.replayed);
    assert_eq!(result.outcome.candidate.status, CandidateStatus::Accepted);
    assert_eq!(result.outcome.decision.actor, "local-user");
    assert_eq!(result.identity.canonical_entity_id, f.ikhwan_b);

    let a = f.service.identity(f.graph_id, f.ikhwan_a).await.unwrap();
    assert_eq!(a.canonical_entity_id, f.ikhwan_b);
    assert!(!a.is_canonical());
    assert!(a.resolved());
    let mut members = vec![f.ikhwan_a, f.ikhwan_b];
    members.sort();
    assert_eq!(a.member_ids(), members);
    let b = f.service.identity(f.graph_id, f.ikhwan_b).await.unwrap();
    assert!(b.is_canonical());
    let alone = f.service.identity(f.graph_id, f.dewi_a).await.unwrap();
    assert!(!alone.resolved());
    assert_eq!(alone.canonical_entity_id, f.dewi_a);

    // The graph is exactly as it was: both entities, their properties and
    // their relationships; the other graph too. No write call was made.
    assert_eq!(snapshot(&f.graph, f.graph_id).await, before);
    assert_eq!(snapshot(&f.graph, f.other_graph).await, other_before);
    assert_eq!(f.graph.writes(), f.setup_writes);
}

#[tokio::test]
async fn decisions_are_validated_idempotent_and_never_overwritten() {
    let f = fixture().await;
    f.service.scan(f.graph_id, customer_scan()).await.unwrap();
    let candidate = candidate_for(&f, f.ikhwan_a, f.ikhwan_b).await;

    let missing_canonical = DecisionRequest {
        canonical_entity_id: None,
        ..same(f.ikhwan_a)
    };
    assert!(matches!(
        resolution_error(
            f.service
                .decide(f.graph_id, candidate, missing_canonical)
                .await
        ),
        ResolutionError::InvalidDecision(_)
    ));
    let rejected_with_canonical = DecisionRequest {
        canonical_entity_id: Some(f.ikhwan_a),
        ..DIFFERENT
    };
    assert!(matches!(
        resolution_error(
            f.service
                .decide(f.graph_id, candidate, rejected_with_canonical)
                .await
        ),
        ResolutionError::InvalidDecision(_)
    ));
    assert!(matches!(
        resolution_error(
            f.service
                .decide(f.graph_id, candidate, same(f.dewi_a))
                .await
        ),
        ResolutionError::IdentityConflict(_)
    ));

    f.service
        .decide(f.graph_id, candidate, same(f.ikhwan_a))
        .await
        .unwrap();
    let replay = f
        .service
        .decide(f.graph_id, candidate, same(f.ikhwan_a))
        .await
        .unwrap();
    assert!(replay.outcome.replayed);
    assert!(matches!(
        resolution_error(f.service.decide(f.graph_id, candidate, DIFFERENT).await),
        ResolutionError::DecisionConflict(_)
    ));
    let detail = f
        .service
        .get_candidate(f.graph_id, candidate)
        .await
        .unwrap();
    assert_eq!(detail.decisions.len(), 1);
}

#[tokio::test]
async fn rejected_pairs_stay_rejected_until_explicitly_reconsidered() {
    let f = fixture().await;
    f.service.scan(f.graph_id, customer_scan()).await.unwrap();
    let dewi = candidate_for(&f, f.dewi_a, f.dewi_b).await;
    f.service.decide(f.graph_id, dewi, DIFFERENT).await.unwrap();
    assert!(
        !f.service
            .identity(f.graph_id, f.dewi_a)
            .await
            .unwrap()
            .resolved()
    );

    let rescan = f.service.scan(f.graph_id, customer_scan()).await.unwrap();
    assert_eq!(rescan.candidates_found, 0);
    assert_eq!(
        rescan.skipped,
        SkippedPairs {
            already_pending: 1,
            previously_rejected: 1,
            already_linked: 0,
        }
    );
    let reconsidered = f
        .service
        .decide(
            f.graph_id,
            dewi,
            DecisionRequest {
                reconsider: true,
                ..same(f.dewi_a)
            },
        )
        .await
        .unwrap();
    assert!(reconsidered.outcome.decision.reconsidered);
    let history = f
        .service
        .get_candidate(f.graph_id, dewi)
        .await
        .unwrap()
        .decisions;
    assert_eq!(history.len(), 2, "the earlier rejection stays visible");
}

#[tokio::test]
async fn candidate_details_read_both_original_entities() {
    let f = fixture().await;
    f.service.scan(f.graph_id, customer_scan()).await.unwrap();
    let candidate = candidate_for(&f, f.ikhwan_a, f.ikhwan_b).await;
    let detail = f
        .service
        .get_candidate(f.graph_id, candidate)
        .await
        .unwrap();
    let names: BTreeSet<String> = [&detail.entity_a, &detail.entity_b]
        .into_iter()
        .map(
            |entity| match entity.as_ref().unwrap().properties().get("name") {
                Some(PropertyValue::String(name)) => name.clone(),
                other => panic!("{other:?}"),
            },
        )
        .collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "Muhammad Ikhwan Nasution".to_owned(),
            "M. Ikhwan Nasution".to_owned()
        ])
    );
    assert!(!detail.identity_a.resolved() && !detail.identity_b.resolved());
    assert!(detail.decisions.is_empty());
}

#[tokio::test]
async fn graphs_are_isolated() {
    let f = fixture().await;
    f.service.scan(f.graph_id, customer_scan()).await.unwrap();
    let candidate = candidate_for(&f, f.ikhwan_a, f.ikhwan_b).await;
    assert!(matches!(
        resolution_error(f.service.get_candidate(f.other_graph, candidate).await),
        ResolutionError::CandidateNotFound(_)
    ));
    assert!(matches!(
        resolution_error(
            f.service
                .decide(f.other_graph, candidate, same(f.ikhwan_a))
                .await
        ),
        ResolutionError::CandidateNotFound(_)
    ));
    let other = f
        .service
        .list_candidates(f.other_graph, query())
        .await
        .unwrap();
    assert!(other.candidates.is_empty());
    // An entity of this graph is missing from the other one.
    assert!(matches!(
        f.service.identity(f.other_graph, f.ikhwan_a).await,
        Err(ApplicationError::EntityNotFound(_))
    ));
    assert!(matches!(
        resolution_error(
            f.service
                .get_scan(f.other_graph, ResolutionScanId::new())
                .await
        ),
        ResolutionError::ScanNotFound(_)
    ));
}

#[tokio::test]
async fn entities_of_different_types_cannot_share_an_identity() {
    let f = fixture().await;
    // A candidate stored by a (hypothetically faulty) writer that pairs a
    // customer with a merchant.
    let (a, b) = if f.ikhwan_a < f.merchant {
        (f.ikhwan_a, f.merchant)
    } else {
        (f.merchant, f.ikhwan_a)
    };
    let candidate = ResolutionCandidateId::new();
    let rules = f
        .service
        .scan(f.graph_id, customer_scan())
        .await
        .unwrap()
        .rules;
    f.store
        .create_scan_with_candidates(
            ResolutionScan {
                id: ResolutionScanId::new(),
                graph_id: f.graph_id,
                entity_type: TypeName::new("Customer").unwrap(),
                rules,
                ontology_version: OntologyVersion::new(2).unwrap(),
                max_entities: 400,
                entities_examined: 2,
                truncated: false,
                pairs_compared: 1,
                comparison_budget: 20_000,
                comparison_budget_reached: false,
                oversized_blocks: 0,
                candidate_limit_reached: false,
                candidates_found: 0,
                skipped: SkippedPairs::default(),
                warnings: vec![],
                duration_ms: 1,
                created_at: chrono::Utc::now(),
            },
            vec![NewCandidate {
                id: candidate,
                entity_a: a,
                entity_b: b,
                score: 0.6,
                signals: vec![],
            }],
        )
        .await
        .unwrap();
    assert!(matches!(
        resolution_error(
            f.service
                .decide(f.graph_id, candidate, same(f.ikhwan_a))
                .await
        ),
        ResolutionError::IdentityConflict(_)
    ));
}

#[tokio::test]
async fn a_missing_entity_blocks_approval() {
    let f = fixture().await;
    f.service.scan(f.graph_id, customer_scan()).await.unwrap();
    let candidate = candidate_for(&f, f.dewi_a, f.dewi_b).await;
    // Removed outside the resolution workflow (test setup only).
    f.graph.delete_entity(f.graph_id, f.dewi_b).await.unwrap();
    assert!(matches!(
        resolution_error(f.service.decide(f.graph_id, candidate, same(f.dewi_a)).await),
        ResolutionError::EntityUnavailable(id) if id == f.dewi_b
    ));
    let detail = f
        .service
        .get_candidate(f.graph_id, candidate)
        .await
        .unwrap();
    assert!(detail.entity_a.is_some() != detail.entity_b.is_some());
}

#[tokio::test]
async fn candidate_pages_are_bounded() {
    let f = fixture().await;
    for limit in [0, 101] {
        let result = f
            .service
            .list_candidates(f.graph_id, CandidateQuery { limit, ..query() })
            .await;
        assert!(
            matches!(result, Err(ApplicationError::InvalidRequest(_))),
            "{limit}"
        );
    }
    assert!(matches!(
        f.service.list_scans(f.graph_id, Some(0)).await,
        Err(ApplicationError::InvalidRequest(_))
    ));
}
