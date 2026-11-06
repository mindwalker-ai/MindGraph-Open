//! Scan-level matching behaviour with synthetic customers. No network,
//! PostgreSQL or HugeGraph.

#![allow(clippy::unwrap_used)] // Panicking is the intended failure mode in tests.

use mindgraph_domain::{
    Entity, EntityId, EntityType, EntityTypeId, GraphId, Properties, PropertiesSchema,
    PropertyDefinition, PropertyKey, PropertyType, PropertyValue, TypeName,
};
use mindgraph_entity_resolution::{
    RuleKind, RuleSet, RuleSpec, ScanLimits, SignalNote, SignalOutcome, WarningCode,
    compare_entities, find_candidates,
};

fn customer_type() -> EntityType {
    let property = |name: &str, t| PropertyDefinition::optional(PropertyKey::new(name).unwrap(), t);
    EntityType::new(
        EntityTypeId::new(),
        GraphId::new(),
        TypeName::new("Customer").unwrap(),
        PropertiesSchema::new(vec![
            property("customer_id", PropertyType::String),
            property("name", PropertyType::String),
            property("email", PropertyType::String),
            property("phone", PropertyType::String),
            property("legacy_phone", PropertyType::Integer),
            property("city", PropertyType::String),
        ])
        .unwrap(),
    )
}

fn entity(entity_type: &str, values: &[(&str, PropertyValue)]) -> Entity {
    let mut properties = Properties::new();
    for (key, value) in values {
        properties
            .insert(PropertyKey::new(*key).unwrap(), value.clone())
            .unwrap();
    }
    Entity::new(
        EntityId::new(),
        GraphId::new(),
        TypeName::new(entity_type).unwrap(),
        properties,
    )
}

fn customer(values: &[(&str, &str)]) -> Entity {
    let values: Vec<(&str, PropertyValue)> = values
        .iter()
        .map(|(k, v)| (*k, PropertyValue::from(*v)))
        .collect();
    entity("Customer", &values)
}

fn spec(property: &str, kind: &str) -> RuleSpec {
    RuleSpec {
        property: property.into(),
        kind: kind.into(),
        ..RuleSpec::default()
    }
}

fn rules(specs: &[RuleSpec]) -> RuleSet {
    RuleSet::validate(&customer_type(), specs).unwrap()
}

fn all_rules() -> RuleSet {
    rules(&[
        spec("customer_id", "exact_identifier"),
        spec("email", "normalized_email"),
        RuleSpec {
            country: Some("ID".into()),
            ..spec("phone", "normalized_phone")
        },
        spec("name", "fuzzy_name"),
    ])
}

fn outcome(found: &mindgraph_entity_resolution::CandidateMatch, property: &str) -> SignalOutcome {
    found
        .signals
        .iter()
        .find(|signal| signal.property == property)
        .unwrap()
        .outcome
}

#[test]
fn the_spec_example_is_a_strong_candidate_with_explained_evidence() {
    let a = customer(&[
        ("name", "Muhammad Ikhwan Nasution"),
        ("email", "ikhwan@example.com"),
        ("phone", "+6281212345678"),
    ]);
    let b = customer(&[
        ("name", "M. Ikhwan Nasution"),
        ("email", "IKHWAN@example.com"),
        ("phone", "081212345678"),
    ]);
    let found = compare_entities(&all_rules(), &a, &b).unwrap();
    assert_eq!(outcome(&found, "email"), SignalOutcome::Normalized);
    assert_eq!(outcome(&found, "phone"), SignalOutcome::Normalized);
    assert_eq!(outcome(&found, "name"), SignalOutcome::Fuzzy);
    assert_eq!(outcome(&found, "customer_id"), SignalOutcome::Missing);
    let email = found
        .signals
        .iter()
        .find(|s| s.property == "email")
        .unwrap();
    assert_eq!(email.note, Some(SignalNote::CaseInsensitiveLocalPart));
    let name = found.signals.iter().find(|s| s.property == "name").unwrap();
    assert_eq!(name.note, Some(SignalNote::InitialsMatched));
    assert!(found.score > 0.95 && found.score < 1.0, "{}", found.score);
    assert_eq!(found.matched_rules().count(), 3);
}

#[test]
fn exact_identifiers_match_and_different_ones_do_not() {
    let rules = rules(&[spec("customer_id", "exact_identifier")]);
    let a = customer(&[("customer_id", "C001")]);
    let b = customer(&[("customer_id", " C001 ")]);
    let c = customer(&[("customer_id", "c001")]);
    let found = compare_entities(&rules, &a, &b).unwrap();
    assert_eq!(found.signals[0].outcome, SignalOutcome::Exact);
    assert_eq!(found.score, 0.95);
    // Identifiers are case-sensitive: no normalisation is assumed.
    assert!(compare_entities(&rules, &a, &c).is_none());
}

#[test]
fn integer_identifiers_compare_by_value() {
    let entity_type = customer_type();
    let rules =
        RuleSet::validate(&entity_type, &[spec("legacy_phone", "exact_identifier")]).unwrap();
    let a = entity("Customer", &[("legacy_phone", PropertyValue::Integer(812))]);
    let b = entity("Customer", &[("legacy_phone", PropertyValue::Integer(812))]);
    assert!(compare_entities(&rules, &a, &b).is_some());
}

#[test]
fn normalised_email_and_phone_match() {
    let rules = all_rules();
    let a = customer(&[
        ("email", " budi@Example.com"),
        ("phone", "+62 813-9876-5432"),
    ]);
    let b = customer(&[("email", "budi@example.COM "), ("phone", "0813 9876 5432")]);
    let found = compare_entities(&rules, &a, &b).unwrap();
    let email = found
        .signals
        .iter()
        .find(|s| s.property == "email")
        .unwrap();
    assert_eq!(email.outcome, SignalOutcome::Normalized);
    assert_eq!(email.note, None, "same local part: the full-weight match");
    assert_eq!(email.weight, 0.9);
    assert_eq!(outcome(&found, "phone"), SignalOutcome::Normalized);
}

#[test]
fn case_insensitive_local_parts_can_be_turned_off() {
    let rules = rules(&[RuleSpec {
        case_insensitive_local_part: Some(false),
        ..spec("email", "normalized_email")
    }]);
    let a = customer(&[("email", "Ikhwan@example.com")]);
    let b = customer(&[("email", "ikhwan@example.com")]);
    assert!(compare_entities(&rules, &a, &b).is_none());
}

#[test]
fn fuzzy_names_respect_the_threshold() {
    let a = customer(&[("name", "Budi Santoso")]);
    let b = customer(&[("name", "Budi Santosa")]);
    let strict = rules(&[RuleSpec {
        threshold: Some(1.0),
        ..spec("name", "fuzzy_name")
    }]);
    assert!(
        compare_entities(&strict, &a, &b).is_none(),
        "1.0 demands equal names"
    );
    let default = rules(&[spec("name", "fuzzy_name")]);
    let found = compare_entities(&default, &a, &b).unwrap();
    let signal = &found.signals[0];
    assert_eq!(signal.outcome, SignalOutcome::Fuzzy);
    assert!(signal.similarity.unwrap() >= 0.9);
    // The weight scales with the similarity, so a name alone stays weak.
    assert!(found.score < 0.6);
}

#[test]
fn common_names_are_weak_candidates_with_visible_disagreement() {
    // Two different people who share a common name.
    let a = customer(&[
        ("name", "Dewi Kusuma"),
        ("email", "dewi.k@example.org"),
        ("phone", "+62 811 2222 3333"),
    ]);
    let b = customer(&[
        ("name", "Dewi Kusuma"),
        ("email", "dkusuma@example.net"),
        ("phone", "+62 812 9999 0000"),
    ]);
    let found = compare_entities(&all_rules(), &a, &b).unwrap();
    assert_eq!(outcome(&found, "name"), SignalOutcome::Exact);
    assert_eq!(outcome(&found, "email"), SignalOutcome::NoMatch);
    assert_eq!(outcome(&found, "phone"), SignalOutcome::NoMatch);
    assert_eq!(found.score, 0.6, "a name alone is weak evidence");

    // A strong candidate ranks above it.
    let c = customer(&[("name", "Dewi K."), ("email", "dewi.k@example.org")]);
    let strong = compare_entities(&all_rules(), &a, &c).unwrap();
    assert!(strong.score > found.score);
}

#[test]
fn a_very_common_name_block_is_skipped_and_reported() {
    let rules = rules(&[spec("name", "fuzzy_name")]);
    let entities: Vec<Entity> = (0..60).map(|_| customer(&[("name", "Budi")])).collect();
    let report = find_candidates(&entities, &rules, &ScanLimits::default());
    assert!(report.candidates.is_empty());
    assert_eq!(report.oversized_blocks, 1);
    assert_eq!(report.warnings[0].code, WarningCode::OversizedBlocks);
    assert!(
        !report.complete(),
        "a skipped block is not an exhaustive scan"
    );
}

#[test]
fn missing_and_blank_values_never_match() {
    let rules = all_rules();
    let a = customer(&[("name", "   "), ("email", "")]);
    let b = customer(&[("name", ""), ("email", "  ")]);
    assert!(compare_entities(&rules, &a, &b).is_none());
    let c = customer(&[]);
    let d = customer(&[]);
    assert!(compare_entities(&rules, &c, &d).is_none());
    let report = find_candidates(&[a, b, c, d], &rules, &ScanLimits::default());
    assert!(report.candidates.is_empty());
    assert_eq!(
        report.pairs_compared, 0,
        "missing values are never blocked together"
    );
}

#[test]
fn integer_phones_are_unavailable_not_reconstructed() {
    let entity_type = customer_type();
    let rules = RuleSet::validate(
        &entity_type,
        &[
            RuleSpec {
                country: Some("ID".into()),
                ..spec("legacy_phone", "normalized_phone")
            },
            spec("name", "fuzzy_name"),
        ],
    )
    .unwrap();
    // 812… stored as an integer lost its leading zero.
    let a = entity(
        "Customer",
        &[
            ("legacy_phone", PropertyValue::Integer(81_212_345_678)),
            ("name", "Alice Tan".into()),
        ],
    );
    let b = entity(
        "Customer",
        &[
            ("legacy_phone", PropertyValue::Integer(81_212_345_678)),
            ("name", "Alice Tan".into()),
        ],
    );
    let report = find_candidates(&[a, b], &rules, &ScanLimits::default());
    let found = &report.candidates[0];
    let phone = &found.signals[0];
    assert_eq!(phone.outcome, SignalOutcome::Unavailable);
    assert_eq!(phone.note, Some(SignalNote::IntegerPhone));
    assert!(!phone.matched);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::IntegerPhone && w.count == 2)
    );
}

#[test]
fn national_phones_without_a_country_are_reported() {
    let rules = rules(&[spec("phone", "normalized_phone")]);
    let a = customer(&[("phone", "0812 1234 5678")]);
    let b = customer(&[("phone", "0812 1234 5678")]);
    let report = find_candidates(&[a, b], &rules, &ScanLimits::default());
    assert!(report.candidates.is_empty());
    assert_eq!(report.warnings[0].code, WarningCode::MissingCountry);
    assert_eq!(report.warnings[0].count, 2);
}

#[test]
fn entities_of_other_types_are_never_compared() {
    let rules = rules(&[spec("name", "fuzzy_name")]);
    let customer = customer(&[("name", "Acme Store")]);
    let merchant = entity("Merchant", &[("name", "Acme Store".into())]);
    assert!(compare_entities(&rules, &customer, &merchant).is_none());
    let report = find_candidates(&[customer, merchant], &rules, &ScanLimits::default());
    assert_eq!(report.entities_examined, 1);
    assert!(report.candidates.is_empty());
}

#[test]
fn scans_are_deterministic_and_ordered() {
    let rules = all_rules();
    let entities = vec![
        customer(&[("name", "Alice Tan"), ("email", "alice@example.com")]),
        customer(&[("name", "Alice  TAN"), ("email", "Alice@example.com")]),
        customer(&[("name", "A. Tan"), ("phone", "0812 0000 1111")]),
        customer(&[("name", "Alicia Tan"), ("phone", "+62 812 0000 1111")]),
        customer(&[("name", "Budi Santoso")]),
        customer(&[("name", "Budi Santosa")]),
    ];
    let first = find_candidates(&entities, &rules, &ScanLimits::default());
    let mut reversed = entities.clone();
    reversed.reverse();
    let second = find_candidates(&reversed, &rules, &ScanLimits::default());
    assert_eq!(first, second, "input order does not matter");
    assert!(first.candidates.windows(2).all(|pair| {
        pair[0].score > pair[1].score
            || (pair[0].score == pair[1].score
                && (pair[0].entity_a, pair[0].entity_b) < (pair[1].entity_a, pair[1].entity_b))
    }));
    assert!(first.candidates.iter().all(|c| c.entity_a < c.entity_b));
}

#[test]
fn a_pair_found_by_several_blocks_is_compared_once() {
    let rules = all_rules();
    let a = customer(&[
        ("name", "Alice Tan"),
        ("email", "alice@example.com"),
        ("phone", "+62 812 0000 1111"),
    ]);
    let b = customer(&[
        ("name", "Alice Tan"),
        ("email", "alice@example.com"),
        ("phone", "0812 0000 1111"),
    ]);
    let report = find_candidates(&[a, b], &rules, &ScanLimits::default());
    assert_eq!(report.pairs_compared, 1);
    assert_eq!(report.candidates.len(), 1);
}

#[test]
fn the_comparison_budget_bounds_the_scan_and_is_reported() {
    let rules = rules(&[spec("city", "normalized_string")]);
    // 40 entities in one block: 780 pairs.
    let entities: Vec<Entity> = (0..40).map(|_| customer(&[("city", "Jakarta")])).collect();
    let limits = ScanLimits {
        max_comparisons: 100,
        ..ScanLimits::default()
    };
    let report = find_candidates(&entities, &rules, &limits);
    assert_eq!(report.pairs_compared, 100);
    assert!(report.comparison_budget_reached);
    assert_eq!(report.candidates.len(), 100);
    assert!(!report.complete());

    let capped = find_candidates(
        &entities,
        &rules,
        &ScanLimits {
            max_candidates: 10,
            ..ScanLimits::default()
        },
    );
    assert_eq!(capped.candidates.len(), 10);
    assert!(capped.candidate_limit_reached);
}

#[test]
fn strong_rules_are_discovered_before_fuzzy_ones() {
    // With a budget of one comparison, the email pair wins over the name pair.
    let rules = rules(&[
        spec("name", "fuzzy_name"),
        spec("email", "normalized_email"),
    ]);
    let a = customer(&[("name", "Zed Zulu")]);
    let b = customer(&[("name", "Zed Zulu")]);
    let c = customer(&[("email", "x@example.com")]);
    let d = customer(&[("email", "x@example.com")]);
    let (c_id, d_id) = (c.id(), d.id());
    let report = find_candidates(
        &[a, b, c, d],
        &rules,
        &ScanLimits {
            max_comparisons: 1,
            ..ScanLimits::default()
        },
    );
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(
        (report.candidates[0].entity_a, report.candidates[0].entity_b),
        (c_id, d_id)
    );
    assert_eq!(
        report.candidates[0].signals[1].rule,
        RuleKind::NormalizedEmail
    );
}
