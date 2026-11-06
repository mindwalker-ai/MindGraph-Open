//! The behavioural contract of `EntityResolutionStore` (M9), shared by every
//! backend: `MemoryEntityResolutionStore` (the reference) and
//! `PostgresEntityResolutionStore`.
//!
//! Each case receives a fresh metadata store and resolution store over the
//! same database. Backends opt in with `resolution_store_contract!`, which
//! generates one multi-threaded `#[tokio::test]` per case.

#![allow(clippy::unwrap_used, clippy::too_many_lines)] // Panicking is the intended failure mode in tests.

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{DateTime, Duration, SubsecRound, Utc};
use mindgraph_core::matching::{
    MatchSignal, RuleKind, RuleSet, RuleSpec, ScanWarning, SignalOutcome, WarningCode,
};
use mindgraph_core::{
    CandidateQuery, CandidateStatus, DecisionCommand, DecisionKind, EntityResolutionStore,
    LOCAL_ACTOR, MetadataStore, NewCandidate, ResolutionScan, ResolutionStoreError, SkippedPairs,
};
use mindgraph_domain::{
    EntityId, EntityType, EntityTypeId, GraphId, GraphProject, GraphStatus, Ontology,
    OntologyVersion, PropertiesSchema, PropertyDefinition, PropertyKey, PropertyType,
    ResolutionCandidateId, ResolutionDecisionId, ResolutionScanId, TypeName,
};

/// The stores of one case.
pub struct Stores {
    pub metadata: Arc<dyn MetadataStore>,
    pub resolution: Arc<dyn EntityResolutionStore>,
}

/// One test per case for `$factory`, an `async fn() -> Stores`. The bracket
/// holds extra attributes, e.g. `[ignore = "needs PostgreSQL"]`.
macro_rules! resolution_store_contract {
    ($attrs:tt $factory:path) => {
        resolution_store_contract!(@cases $attrs $factory;
            scans_round_trip_with_their_candidates,
            candidates_are_ordered_paginated_filtered_and_counted,
            an_invalid_candidate_rolls_back_the_whole_scan,
            accepting_creates_a_group_with_the_chosen_canonical_and_provenance,
            rejecting_records_the_decision_without_a_mapping,
            repeated_decisions_are_idempotent_and_changes_conflict,
            a_rejection_is_reconsidered_only_explicitly,
            groups_consolidate_transitively_under_one_canonical,
            the_canonical_entity_must_belong_to_the_group,
            linking_cannot_contradict_an_earlier_rejection,
            linked_entities_cannot_be_rejected,
            group_size_is_bounded,
            groups_never_mix_entity_types,
            rescans_skip_pending_rejected_and_linked_pairs,
            graphs_are_isolated,
            unknown_cursors_are_rejected,
            concurrent_decisions_keep_one_canonical_per_group,
            concurrent_conflicting_decisions_let_exactly_one_win,
        );
    };
    (@cases $attrs:tt $factory:path; $($case:ident),* $(,)?) => {
        $( resolution_store_contract!(@case $attrs $factory; $case); )*
    };
    (@case [$($attr:meta),*] $factory:path; $case:ident) => {
        $(#[$attr])*
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn $case() {
            let stores = $factory().await;
            resolution_contract::$case(stores).await;
        }
    };
}

// --- fixtures -------------------------------------------------------------------------------

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn entity_type(graph_id: GraphId, name: &str) -> EntityType {
    let property = |key: &str| {
        PropertyDefinition::optional(PropertyKey::new(key).unwrap(), PropertyType::String)
    };
    EntityType::new(
        EntityTypeId::new(),
        graph_id,
        TypeName::new(name).unwrap(),
        PropertiesSchema::new(vec![property("name"), property("email")]).unwrap(),
    )
}

/// A graph whose ontology v1 has `Customer` and `Account`.
async fn graph(metadata: &dyn MetadataStore) -> GraphId {
    let workspace = metadata.ensure_local_workspace().await.unwrap();
    let at = now();
    let graph = GraphProject::from_parts(
        GraphId::new(),
        workspace.id(),
        "Resolution contract",
        None,
        GraphStatus::Draft,
        at,
        at,
    )
    .unwrap();
    let ontology = Ontology::new(
        graph.id(),
        OntologyVersion::INITIAL,
        vec![
            entity_type(graph.id(), "Customer"),
            entity_type(graph.id(), "Account"),
        ],
        vec![],
    )
    .unwrap();
    metadata.create_graph(&graph, &ontology).await.unwrap();
    graph.id()
}

fn rules(graph_id: GraphId, type_name: &str) -> RuleSet {
    let spec = |property: &str, kind: &str| RuleSpec {
        property: property.into(),
        kind: kind.into(),
        ..RuleSpec::default()
    };
    RuleSet::validate(
        &entity_type(graph_id, type_name),
        &[
            spec("email", "normalized_email"),
            spec("name", "fuzzy_name"),
        ],
    )
    .unwrap()
}

fn scan_of(graph_id: GraphId, type_name: &str) -> ResolutionScan {
    ResolutionScan {
        id: ResolutionScanId::new(),
        graph_id,
        entity_type: TypeName::new(type_name).unwrap(),
        rules: rules(graph_id, type_name),
        ontology_version: OntologyVersion::INITIAL,
        max_entities: 400,
        entities_examined: 12,
        truncated: true,
        pairs_compared: 30,
        comparison_budget: 20_000,
        comparison_budget_reached: false,
        oversized_blocks: 1,
        candidate_limit_reached: false,
        candidates_found: 0,
        skipped: SkippedPairs::default(),
        warnings: vec![ScanWarning {
            code: WarningCode::InvalidValues,
            property: "email".into(),
            rule: RuleKind::NormalizedEmail,
            count: 2,
        }],
        duration_ms: 42,
        created_at: now(),
    }
}

fn scan(graph_id: GraphId) -> ResolutionScan {
    scan_of(graph_id, "Customer")
}

fn signals() -> Vec<MatchSignal> {
    vec![
        MatchSignal {
            property: "email".into(),
            rule: RuleKind::NormalizedEmail,
            outcome: SignalOutcome::Normalized,
            matched: true,
            similarity: None,
            weight: 0.9,
            note: None,
        },
        MatchSignal {
            property: "name".into(),
            rule: RuleKind::FuzzyName,
            outcome: SignalOutcome::NoMatch,
            matched: false,
            similarity: Some(0.71),
            weight: 0.0,
            note: None,
        },
    ]
}

fn pair(a: EntityId, b: EntityId, score: f64) -> NewCandidate {
    let (entity_a, entity_b) = if a < b { (a, b) } else { (b, a) };
    NewCandidate {
        id: ResolutionCandidateId::new(),
        entity_a,
        entity_b,
        score,
        signals: signals(),
    }
}

fn entities<const N: usize>() -> [EntityId; N] {
    let mut ids = [EntityId::new(); N];
    for id in &mut ids {
        *id = EntityId::new();
    }
    ids
}

fn decide(
    graph_id: GraphId,
    candidate: ResolutionCandidateId,
    kind: DecisionKind,
    canonical: Option<EntityId>,
) -> DecisionCommand {
    DecisionCommand {
        id: ResolutionDecisionId::new(),
        graph_id,
        candidate_id: candidate,
        kind,
        canonical,
        reconsider: false,
        actor: LOCAL_ACTOR.into(),
        decided_at: now(),
        max_group_members: 100,
    }
}

fn same(
    graph_id: GraphId,
    candidate: ResolutionCandidateId,
    canonical: EntityId,
) -> DecisionCommand {
    decide(
        graph_id,
        candidate,
        DecisionKind::SameEntity,
        Some(canonical),
    )
}

fn different(graph_id: GraphId, candidate: ResolutionCandidateId) -> DecisionCommand {
    decide(graph_id, candidate, DecisionKind::DifferentEntities, None)
}

fn query() -> CandidateQuery {
    CandidateQuery {
        status: None,
        scan_id: None,
        entity_id: None,
        limit: 100,
        after: None,
    }
}

/// Stores one scan with `pairs` and returns the candidate ids in input order.
async fn stored(
    store: &dyn EntityResolutionStore,
    graph_id: GraphId,
    pairs: Vec<NewCandidate>,
) -> Vec<ResolutionCandidateId> {
    let ids = pairs.iter().map(|p| p.id).collect();
    store
        .create_scan_with_candidates(scan(graph_id), pairs)
        .await
        .unwrap();
    ids
}

fn identity_conflict(result: Result<impl std::fmt::Debug, ResolutionStoreError>) {
    assert!(
        matches!(result, Err(ResolutionStoreError::IdentityConflict(_))),
        "{result:?}"
    );
}

fn decision_conflict(result: Result<impl std::fmt::Debug, ResolutionStoreError>) {
    assert!(
        matches!(result, Err(ResolutionStoreError::DecisionConflict(_))),
        "{result:?}"
    );
}

async fn canonical(
    store: &dyn EntityResolutionStore,
    graph_id: GraphId,
    entity: EntityId,
) -> EntityId {
    store
        .resolve_canonical_identity(graph_id, entity)
        .await
        .unwrap()
}

// --- cases ----------------------------------------------------------------------------------

pub async fn scans_round_trip_with_their_candidates(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b, c] = entities();
    let scan = scan(graph_id);
    let stored = stores
        .resolution
        .create_scan_with_candidates(scan.clone(), vec![pair(a, b, 0.985), pair(b, c, 0.57)])
        .await
        .unwrap();
    assert_eq!(stored.candidates_found, 2);
    assert_eq!(stored.skipped, SkippedPairs::default());
    let read = stores
        .resolution
        .get_scan(graph_id, scan.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read, stored,
        "every field round-trips, rule snapshot included"
    );
    assert_eq!(
        read.rules.labels(),
        ["normalized_email:email", "fuzzy_name:name"]
    );
    assert!(!read.exhaustive(), "truncated scans are never exhaustive");

    let page = stores
        .resolution
        .list_candidates(graph_id, &query())
        .await
        .unwrap();
    assert_eq!(page.candidates.len(), 2);
    let first = &page.candidates[0];
    assert_eq!((first.entity_a, first.entity_b), (a, b));
    assert_eq!(first.status, CandidateStatus::Pending);
    assert_eq!(first.scan_id, scan.id);
    assert_eq!(first.entity_type.as_str(), "Customer");
    assert_eq!(first.signals, signals());
    assert_eq!(first.decided_at, None);
    let fetched = stores
        .resolution
        .get_candidate(graph_id, first.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&fetched, first);

    let scans = stores.resolution.list_scans(graph_id, 10).await.unwrap();
    assert_eq!(scans, vec![stored]);
}

pub async fn candidates_are_ordered_paginated_filtered_and_counted(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b, c, d, e] = entities();
    let pairs = vec![
        pair(a, b, 0.6),
        pair(a, c, 0.95),
        pair(b, c, 0.6),
        pair(c, d, 0.75),
        pair(d, e, 0.6),
    ];
    let mut expected: Vec<(f64, ResolutionCandidateId)> =
        pairs.iter().map(|p| (p.score, p.id)).collect();
    expected.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)));
    let ids = stored(stores.resolution.as_ref(), graph_id, pairs).await;

    // Pages of two follow score DESC, id ASC without gaps or repeats.
    let mut seen = Vec::new();
    let mut after = None;
    loop {
        let page = stores
            .resolution
            .list_candidates(
                graph_id,
                &CandidateQuery {
                    limit: 2,
                    after,
                    ..query()
                },
            )
            .await
            .unwrap();
        assert_eq!(page.counts.pending, 5);
        seen.extend(page.candidates.iter().map(|c| (c.score, c.id)));
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(seen, expected);

    // Filters: entity, status, scan; counts follow the scan/entity filters.
    let involving_c = stores
        .resolution
        .list_candidates(
            graph_id,
            &CandidateQuery {
                entity_id: Some(c),
                ..query()
            },
        )
        .await
        .unwrap();
    assert_eq!(involving_c.candidates.len(), 3);
    assert_eq!(involving_c.counts.pending, 3);

    stores
        .resolution
        .record_decision(different(graph_id, ids[0]))
        .await
        .unwrap();
    let pending = stores
        .resolution
        .list_candidates(
            graph_id,
            &CandidateQuery {
                status: Some(CandidateStatus::Pending),
                ..query()
            },
        )
        .await
        .unwrap();
    assert_eq!(pending.candidates.len(), 4);
    assert_eq!((pending.counts.pending, pending.counts.rejected), (4, 1));
    let other_scan = stores
        .resolution
        .list_candidates(
            graph_id,
            &CandidateQuery {
                scan_id: Some(ResolutionScanId::new()),
                ..query()
            },
        )
        .await
        .unwrap();
    assert!(other_scan.candidates.is_empty());
    assert_eq!(other_scan.counts.pending, 0);
}

pub async fn an_invalid_candidate_rolls_back_the_whole_scan(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b] = entities();
    for bad in [
        vec![pair(a, b, 0.9), pair(a, b, 0.8)],
        vec![NewCandidate {
            entity_a: a,
            entity_b: a,
            ..pair(a, b, 0.9)
        }],
        vec![NewCandidate {
            entity_a: b,
            entity_b: a,
            ..pair(a, b, 0.9)
        }],
    ] {
        let scan = scan(graph_id);
        let result = stores
            .resolution
            .create_scan_with_candidates(scan.clone(), bad)
            .await;
        assert!(
            matches!(result, Err(ResolutionStoreError::InvalidInput(_))),
            "{result:?}"
        );
        assert_eq!(
            stores.resolution.get_scan(graph_id, scan.id).await.unwrap(),
            None
        );
    }
    let page = stores
        .resolution
        .list_candidates(graph_id, &query())
        .await
        .unwrap();
    assert!(page.candidates.is_empty());
    assert!(
        stores
            .resolution
            .list_scans(graph_id, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

pub async fn accepting_creates_a_group_with_the_chosen_canonical_and_provenance(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b] = entities();
    let candidate = pair(a, b, 0.985);
    let scan = scan(graph_id);
    let scan_id = scan.id;
    stores
        .resolution
        .create_scan_with_candidates(scan, vec![candidate.clone()])
        .await
        .unwrap();
    // The reviewer keeps the *newer* entity: no oldest-id policy.
    let command = same(graph_id, candidate.id, b);
    let outcome = stores
        .resolution
        .record_decision(command.clone())
        .await
        .unwrap();
    assert!(!outcome.replayed);
    assert_eq!(outcome.candidate.status, CandidateStatus::Accepted);
    assert_eq!(outcome.candidate.decided_at, Some(command.decided_at));
    let decision = &outcome.decision;
    assert_eq!(decision.id, command.id);
    assert_eq!(decision.kind, DecisionKind::SameEntity);
    assert_eq!((decision.entity_a, decision.entity_b), (a, b));
    assert_eq!(decision.canonical_entity_id, Some(b));
    assert_eq!(decision.scan_id, scan_id);
    assert_eq!(decision.score, 0.985);
    assert_eq!(decision.matched_rules, ["normalized_email:email"]);
    assert_eq!(decision.actor, LOCAL_ACTOR);
    assert!(decision.previous_canonical_ids.is_empty());
    assert_eq!(decision.linked_entity_ids, [a, b]);
    assert!(!decision.reconsidered);

    let group = outcome.group.clone().unwrap();
    assert_eq!(group.group.canonical(), b);
    assert_eq!(group.group.members(), &BTreeSet::from([a, b]));
    assert_eq!(group.entity_type.as_str(), "Customer");
    assert!(group.members.iter().all(|m| m.decision_id == command.id));
    for entity in [a, b] {
        assert_eq!(
            canonical(stores.resolution.as_ref(), graph_id, entity).await,
            b
        );
    }
    // An entity in no group resolves to itself.
    let loner = EntityId::new();
    assert_eq!(
        canonical(stores.resolution.as_ref(), graph_id, loner).await,
        loner
    );
    assert_eq!(
        stores
            .resolution
            .get_identity_group(graph_id, loner)
            .await
            .unwrap(),
        None
    );

    let history = stores
        .resolution
        .list_decisions(graph_id, candidate.id)
        .await
        .unwrap();
    assert_eq!(history, vec![outcome.decision]);
}

pub async fn rejecting_records_the_decision_without_a_mapping(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b] = entities();
    let ids = stored(stores.resolution.as_ref(), graph_id, vec![pair(a, b, 0.6)]).await;
    let outcome = stores
        .resolution
        .record_decision(different(graph_id, ids[0]))
        .await
        .unwrap();
    assert_eq!(outcome.candidate.status, CandidateStatus::Rejected);
    assert_eq!(outcome.decision.kind, DecisionKind::DifferentEntities);
    assert_eq!(outcome.decision.canonical_entity_id, None);
    assert!(outcome.decision.linked_entity_ids.is_empty());
    assert_eq!(outcome.group, None);
    for entity in [a, b] {
        assert_eq!(
            stores
                .resolution
                .get_identity_group(graph_id, entity)
                .await
                .unwrap(),
            None
        );
    }
}

pub async fn repeated_decisions_are_idempotent_and_changes_conflict(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b, c, d] = entities();
    let ids = stored(
        stores.resolution.as_ref(),
        graph_id,
        vec![pair(a, b, 0.9), pair(c, d, 0.6)],
    )
    .await;
    let first = stores
        .resolution
        .record_decision(same(graph_id, ids[0], a))
        .await
        .unwrap();
    let again = stores
        .resolution
        .record_decision(same(graph_id, ids[0], a))
        .await
        .unwrap();
    assert!(again.replayed);
    assert_eq!(again.decision, first.decision);
    decision_conflict(
        stores
            .resolution
            .record_decision(same(graph_id, ids[0], b))
            .await,
    );
    decision_conflict(
        stores
            .resolution
            .record_decision(different(graph_id, ids[0]))
            .await,
    );
    assert_eq!(
        stores
            .resolution
            .list_decisions(graph_id, ids[0])
            .await
            .unwrap()
            .len(),
        1
    );

    stores
        .resolution
        .record_decision(different(graph_id, ids[1]))
        .await
        .unwrap();
    let again = stores
        .resolution
        .record_decision(different(graph_id, ids[1]))
        .await
        .unwrap();
    assert!(again.replayed);
    assert_eq!(
        stores
            .resolution
            .list_decisions(graph_id, ids[1])
            .await
            .unwrap()
            .len(),
        1
    );
}

pub async fn a_rejection_is_reconsidered_only_explicitly(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b] = entities();
    let ids = stored(stores.resolution.as_ref(), graph_id, vec![pair(a, b, 0.6)]).await;
    stores
        .resolution
        .record_decision(different(graph_id, ids[0]))
        .await
        .unwrap();
    decision_conflict(
        stores
            .resolution
            .record_decision(same(graph_id, ids[0], a))
            .await,
    );

    let mut command = same(graph_id, ids[0], a);
    command.reconsider = true;
    command.decided_at = now() + Duration::milliseconds(5);
    let outcome = stores.resolution.record_decision(command).await.unwrap();
    assert!(outcome.decision.reconsidered);
    assert_eq!(outcome.candidate.status, CandidateStatus::Accepted);
    let history = stores
        .resolution
        .list_decisions(graph_id, ids[0])
        .await
        .unwrap();
    let kinds: Vec<DecisionKind> = history.iter().map(|d| d.kind).collect();
    assert_eq!(
        kinds,
        [DecisionKind::DifferentEntities, DecisionKind::SameEntity],
        "the rejection stays in the history"
    );
}

pub async fn groups_consolidate_transitively_under_one_canonical(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [e1, e2, e3, e4] = entities();
    let ids = stored(
        stores.resolution.as_ref(),
        graph_id,
        vec![pair(e1, e2, 0.9), pair(e3, e4, 0.9), pair(e2, e4, 0.8)],
    )
    .await;
    let store = stores.resolution.as_ref();
    store
        .record_decision(same(graph_id, ids[0], e1))
        .await
        .unwrap();
    store
        .record_decision(same(graph_id, ids[1], e3))
        .await
        .unwrap();
    let outcome = store
        .record_decision(same(graph_id, ids[2], e3))
        .await
        .unwrap();
    assert_eq!(outcome.decision.previous_canonical_ids, [e1, e3]);
    assert_eq!(
        outcome.decision.linked_entity_ids,
        [e1, e2],
        "only the moved group is rewritten"
    );
    for entity in [e1, e2, e3, e4] {
        assert_eq!(
            canonical(store, graph_id, entity).await,
            e3,
            "one canonical for all"
        );
    }
    let group = store
        .get_identity_group(graph_id, e1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.group.members(), &BTreeSet::from([e1, e2, e3, e4]));
}

pub async fn the_canonical_entity_must_belong_to_the_group(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b, outsider] = entities();
    let ids = stored(stores.resolution.as_ref(), graph_id, vec![pair(a, b, 0.9)]).await;
    identity_conflict(
        stores
            .resolution
            .record_decision(same(graph_id, ids[0], outsider))
            .await,
    );
    let candidate = stores
        .resolution
        .get_candidate(graph_id, ids[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        candidate.status,
        CandidateStatus::Pending,
        "nothing was written"
    );
    assert!(
        stores
            .resolution
            .list_decisions(graph_id, ids[0])
            .await
            .unwrap()
            .is_empty()
    );
}

pub async fn linking_cannot_contradict_an_earlier_rejection(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [e1, e2, e3] = entities();
    let ids = stored(
        stores.resolution.as_ref(),
        graph_id,
        vec![pair(e1, e3, 0.6), pair(e1, e2, 0.9), pair(e2, e3, 0.9)],
    )
    .await;
    let store = stores.resolution.as_ref();
    store
        .record_decision(different(graph_id, ids[0]))
        .await
        .unwrap();
    store
        .record_decision(same(graph_id, ids[1], e1))
        .await
        .unwrap();
    identity_conflict(store.record_decision(same(graph_id, ids[2], e2)).await);
    assert_eq!(
        canonical(store, graph_id, e3).await,
        e3,
        "e3 stays unlinked"
    );
}

pub async fn linked_entities_cannot_be_rejected(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [e1, e2, e3] = entities();
    let ids = stored(
        stores.resolution.as_ref(),
        graph_id,
        vec![pair(e1, e2, 0.9), pair(e2, e3, 0.9), pair(e1, e3, 0.6)],
    )
    .await;
    let store = stores.resolution.as_ref();
    store
        .record_decision(same(graph_id, ids[0], e1))
        .await
        .unwrap();
    store
        .record_decision(same(graph_id, ids[1], e1))
        .await
        .unwrap();
    identity_conflict(store.record_decision(different(graph_id, ids[2])).await);
    // Accepting the redundant pair is fine and changes nothing.
    let outcome = store
        .record_decision(same(graph_id, ids[2], e1))
        .await
        .unwrap();
    assert!(outcome.decision.linked_entity_ids.is_empty());
}

pub async fn group_size_is_bounded(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [e1, e2, e3] = entities();
    let ids = stored(
        stores.resolution.as_ref(),
        graph_id,
        vec![pair(e1, e2, 0.9), pair(e2, e3, 0.9)],
    )
    .await;
    let mut first = same(graph_id, ids[0], e1);
    first.max_group_members = 2;
    stores.resolution.record_decision(first).await.unwrap();
    let mut second = same(graph_id, ids[1], e1);
    second.max_group_members = 2;
    identity_conflict(stores.resolution.record_decision(second).await);
}

pub async fn groups_never_mix_entity_types(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [customer_a, customer_b, account] = entities();
    let ids = stored(
        stores.resolution.as_ref(),
        graph_id,
        vec![pair(customer_a, customer_b, 0.9)],
    )
    .await;
    stores
        .resolution
        .record_decision(same(graph_id, ids[0], customer_a))
        .await
        .unwrap();
    // A scan of another type that (wrongly) pairs a customer with an account.
    let accounts = scan_of(graph_id, "Account");
    let candidate = pair(customer_b, account, 0.9);
    let candidate_id = candidate.id;
    stores
        .resolution
        .create_scan_with_candidates(accounts, vec![candidate])
        .await
        .unwrap();
    identity_conflict(
        stores
            .resolution
            .record_decision(same(graph_id, candidate_id, customer_b))
            .await,
    );
}

pub async fn rescans_skip_pending_rejected_and_linked_pairs(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b, c, d, e, f, g, h] = entities();
    let store = stores.resolution.as_ref();
    let ids = stored(
        store,
        graph_id,
        vec![pair(a, b, 0.9), pair(c, d, 0.6), pair(e, f, 0.9)],
    )
    .await;
    store
        .record_decision(different(graph_id, ids[1]))
        .await
        .unwrap();
    store
        .record_decision(same(graph_id, ids[2], e))
        .await
        .unwrap();

    let rescan = store
        .create_scan_with_candidates(
            scan(graph_id),
            vec![
                pair(a, b, 0.9),
                pair(c, d, 0.6),
                pair(e, f, 0.9),
                pair(g, h, 0.6),
            ],
        )
        .await
        .unwrap();
    assert_eq!(rescan.candidates_found, 1);
    assert_eq!(
        rescan.skipped,
        SkippedPairs {
            already_pending: 1,
            previously_rejected: 1,
            already_linked: 1,
        }
    );
    let pending = store
        .list_candidates(
            graph_id,
            &CandidateQuery {
                status: Some(CandidateStatus::Pending),
                ..query()
            },
        )
        .await
        .unwrap();
    let pairs: BTreeSet<(EntityId, EntityId)> = pending
        .candidates
        .iter()
        .map(|c| (c.entity_a, c.entity_b))
        .collect();
    assert_eq!(
        pairs,
        BTreeSet::from([(a, b), (g, h)]),
        "one pending candidate per pair"
    );
}

pub async fn graphs_are_isolated(stores: Stores) {
    let graph_a = graph(stores.metadata.as_ref()).await;
    let graph_b = graph(stores.metadata.as_ref()).await;
    let [x, y] = entities();
    let scan = scan(graph_a);
    let scan_id = scan.id;
    let candidate = pair(x, y, 0.9);
    let candidate_id = candidate.id;
    let store = stores.resolution.as_ref();
    store
        .create_scan_with_candidates(scan, vec![candidate])
        .await
        .unwrap();

    assert_eq!(store.get_scan(graph_b, scan_id).await.unwrap(), None);
    assert!(store.list_scans(graph_b, 10).await.unwrap().is_empty());
    assert_eq!(
        store.get_candidate(graph_b, candidate_id).await.unwrap(),
        None
    );
    assert!(
        store
            .list_candidates(graph_b, &query())
            .await
            .unwrap()
            .candidates
            .is_empty()
    );
    assert!(matches!(
        store.record_decision(same(graph_b, candidate_id, x)).await,
        Err(ResolutionStoreError::CandidateNotFound(_))
    ));
    store
        .record_decision(same(graph_a, candidate_id, x))
        .await
        .unwrap();
    assert_eq!(store.get_identity_group(graph_b, x).await.unwrap(), None);
    assert_eq!(canonical(store, graph_b, y).await, y);
    assert_eq!(canonical(store, graph_a, y).await, x);
}

pub async fn unknown_cursors_are_rejected(stores: Stores) {
    let graph_a = graph(stores.metadata.as_ref()).await;
    let graph_b = graph(stores.metadata.as_ref()).await;
    let [x, y] = entities();
    let ids = stored(stores.resolution.as_ref(), graph_a, vec![pair(x, y, 0.9)]).await;
    for (graph_id, after) in [(graph_a, ResolutionCandidateId::new()), (graph_b, ids[0])] {
        let result = stores
            .resolution
            .list_candidates(
                graph_id,
                &CandidateQuery {
                    after: Some(after),
                    ..query()
                },
            )
            .await;
        assert!(
            matches!(result, Err(ResolutionStoreError::InvalidCursor)),
            "{result:?}"
        );
    }
}

pub async fn concurrent_decisions_keep_one_canonical_per_group(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let chain: [EntityId; 6] = entities();
    let pairs: Vec<NewCandidate> = chain.windows(2).map(|w| pair(w[0], w[1], 0.9)).collect();
    let ids = stored(stores.resolution.as_ref(), graph_id, pairs).await;

    // Each link approved at the same time, each naming its own canonical
    // entity, plus identical repeats of the first approval.
    let mut tasks = Vec::new();
    for (index, candidate) in ids.iter().enumerate() {
        let store = Arc::clone(&stores.resolution);
        let command = same(graph_id, *candidate, chain[index + 1]);
        tasks.push(tokio::spawn(
            async move { store.record_decision(command).await },
        ));
    }
    for _ in 0..4 {
        let store = Arc::clone(&stores.resolution);
        let command = same(graph_id, ids[0], chain[1]);
        tasks.push(tokio::spawn(
            async move { store.record_decision(command).await },
        ));
    }
    let mut created = 0;
    for task in tasks {
        let outcome = task.await.unwrap().unwrap();
        if !outcome.replayed {
            created += 1;
        }
    }
    assert_eq!(
        created,
        ids.len(),
        "one decision per candidate; repeats replay"
    );
    for candidate in &ids {
        assert_eq!(
            stores
                .resolution
                .list_decisions(graph_id, *candidate)
                .await
                .unwrap()
                .len(),
            1
        );
    }
    let group = stores
        .resolution
        .get_identity_group(graph_id, chain[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.group.len(), chain.len());
    let canonical_entity = group.group.canonical();
    for entity in chain {
        assert_eq!(
            canonical(stores.resolution.as_ref(), graph_id, entity).await,
            canonical_entity,
            "no conflicting canonical mappings"
        );
    }
    assert_eq!(
        canonical(stores.resolution.as_ref(), graph_id, canonical_entity).await,
        canonical_entity,
        "the canonical entity maps to itself"
    );
}

pub async fn concurrent_conflicting_decisions_let_exactly_one_win(stores: Stores) {
    let graph_id = graph(stores.metadata.as_ref()).await;
    let [a, b] = entities();
    let ids = stored(stores.resolution.as_ref(), graph_id, vec![pair(a, b, 0.9)]).await;
    let accept = {
        let store = Arc::clone(&stores.resolution);
        let command = same(graph_id, ids[0], a);
        tokio::spawn(async move { store.record_decision(command).await })
    };
    let reject = {
        let store = Arc::clone(&stores.resolution);
        let command = different(graph_id, ids[0]);
        tokio::spawn(async move { store.record_decision(command).await })
    };
    let results = [accept.await.unwrap(), reject.await.unwrap()];
    let wins = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(wins, 1, "{results:?}");
    assert!(
        results
            .iter()
            .any(|r| matches!(r, Err(ResolutionStoreError::DecisionConflict(_))))
    );
    assert_eq!(
        stores
            .resolution
            .list_decisions(graph_id, ids[0])
            .await
            .unwrap()
            .len(),
        1
    );
}
