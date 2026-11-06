//! Bounded performance sanity check (M9 spec §47): synthetic customers with
//! mixed duplicate patterns, scanned with every rule kind.
//!
//! It prints the measured duration, comparisons and candidates and checks
//! the bounds and the recall of the injected duplicates. It sets no timing
//! SLA: the numbers are recorded in docs/entity-resolution/README.md from a
//! measured run (`cargo test -p mindgraph-entity-resolution --release --test
//! scan_perf -- --nocapture`).

#![allow(clippy::unwrap_used, clippy::cast_precision_loss)] // test-only arithmetic

use std::collections::HashSet;
use std::time::Instant;

use mindgraph_domain::{
    Entity, EntityId, EntityType, EntityTypeId, GraphId, Properties, PropertiesSchema,
    PropertyDefinition, PropertyKey, PropertyType, TypeName,
};
use mindgraph_entity_resolution::{
    CandidateMatch, MatchSignal, RuleSet, RuleSpec, ScanLimits, find_candidates,
};

const FIRST: [&str; 30] = [
    "Ahmad", "Budi", "Citra", "Dewi", "Eko", "Fajar", "Gita", "Hadi", "Indah", "Joko", "Kartika",
    "Lukman", "Maya", "Nanda", "Oka", "Putri", "Rina", "Sari", "Taufik", "Umar", "Vina", "Wahyu",
    "Yusuf", "Zahra", "Agus", "Bayu", "Cahya", "Dian", "Elisa", "Farhan",
];
const LAST: [&str; 30] = [
    "Santoso",
    "Wijaya",
    "Pratama",
    "Kusuma",
    "Lestari",
    "Nasution",
    "Siregar",
    "Hakim",
    "Saputra",
    "Halim",
    "Gunawan",
    "Hidayat",
    "Setiawan",
    "Rahman",
    "Susanto",
    "Utomo",
    "Wibowo",
    "Purnomo",
    "Maulana",
    "Firmansyah",
    "Permana",
    "Suryadi",
    "Ramadhan",
    "Kurniawan",
    "Hasibuan",
    "Tanjung",
    "Harahap",
    "Sinaga",
    "Simanjuntak",
    "Pohan",
];

fn customer_type(graph_id: GraphId) -> EntityType {
    let property = |name: &str| {
        PropertyDefinition::optional(PropertyKey::new(name).unwrap(), PropertyType::String)
    };
    EntityType::new(
        EntityTypeId::new(),
        graph_id,
        TypeName::new("Customer").unwrap(),
        PropertiesSchema::new(vec![
            property("customer_id"),
            property("name"),
            property("email"),
            property("phone"),
        ])
        .unwrap(),
    )
}

fn customer(graph_id: GraphId, values: &[(&str, String)]) -> Entity {
    let mut properties = Properties::new();
    for (key, value) in values {
        properties
            .insert(PropertyKey::new(*key).unwrap(), value.clone())
            .unwrap();
    }
    Entity::new(
        EntityId::new(),
        graph_id,
        TypeName::new("Customer").unwrap(),
        properties,
    )
}

/// `base` distinct customers plus `duplicates` injected near-duplicates of
/// them; returns the entities and the injected pairs.
fn dataset(
    base: usize,
    duplicates: usize,
) -> (GraphId, Vec<Entity>, HashSet<(EntityId, EntityId)>) {
    let graph_id = GraphId::new();
    let mut entities = Vec::with_capacity(base + duplicates);
    let mut people = Vec::with_capacity(base);
    for i in 0..base {
        // A stride coprime with 900 spreads names; some still repeat, like
        // real common names.
        let combo = (i * 37) % (FIRST.len() * LAST.len());
        let (first, last) = (FIRST[combo % FIRST.len()], LAST[combo / FIRST.len()]);
        let email = format!(
            "{}.{}{i}@example.com",
            first.to_lowercase(),
            last.to_lowercase()
        );
        let phone = format!("+62 812 {:04} {:04}", 1000 + i / 100, i % 10_000);
        let entity = customer(
            graph_id,
            &[
                ("customer_id", format!("C{i:05}")),
                ("name", format!("{first} {last}")),
                ("email", email.clone()),
                ("phone", phone.clone()),
            ],
        );
        people.push((entity.id(), first, last, email, phone, i));
        entities.push(entity);
    }
    let mut injected = HashSet::new();
    for k in 0..duplicates {
        let (original, first, last, email, _phone, i) = &people[(k * 7) % base];
        let local_phone = format!("0812-{:04}-{:04}", 1000 + i / 100, i % 10_000);
        let values: Vec<(&str, String)> = match k % 4 {
            // The same email with an upper-case local part.
            0 => vec![
                ("customer_id", format!("K{k:05}")),
                ("name", format!("{first} {last}")),
                (
                    "email",
                    email.to_uppercase().replace("@EXAMPLE.COM", "@example.com"),
                ),
            ],
            // The same phone written in national format.
            1 => vec![
                ("customer_id", format!("K{k:05}")),
                ("name", format!("{} {last}", first.to_uppercase())),
                ("phone", local_phone),
            ],
            // An initial instead of the first name, same email.
            2 => vec![
                ("customer_id", format!("K{k:05}")),
                ("name", format!("{}. {last}", &first[..1])),
                ("email", email.clone()),
            ],
            // Only a typo in the last name: fuzzy evidence alone.
            _ => {
                let mut typo = String::from(*last);
                let end = typo.pop().unwrap();
                typo.push(if end == 'a' { 'o' } else { 'a' });
                vec![
                    ("customer_id", format!("K{k:05}")),
                    ("name", format!("{first} {typo}")),
                ]
            }
        };
        let duplicate = customer(graph_id, &values);
        let pair = if *original < duplicate.id() {
            (*original, duplicate.id())
        } else {
            (duplicate.id(), *original)
        };
        injected.insert(pair);
        entities.push(duplicate);
    }
    (graph_id, entities, injected)
}

fn rules(graph_id: GraphId) -> RuleSet {
    let spec = |property: &str, kind: &str| RuleSpec {
        property: property.into(),
        kind: kind.into(),
        ..RuleSpec::default()
    };
    RuleSet::validate(
        &customer_type(graph_id),
        &[
            spec("customer_id", "exact_identifier"),
            spec("email", "normalized_email"),
            RuleSpec {
                country: Some("ID".into()),
                ..spec("phone", "normalized_phone")
            },
            spec("name", "fuzzy_name"),
        ],
    )
    .unwrap()
}

fn run(base: usize, duplicates: usize) {
    let (graph_id, entities, injected) = dataset(base, duplicates);
    let rules = rules(graph_id);
    let limits = ScanLimits::default();
    let started = Instant::now();
    let report = find_candidates(&entities, &rules, &limits);
    let elapsed = started.elapsed();

    let found: HashSet<(EntityId, EntityId)> = report
        .candidates
        .iter()
        .map(|c| (c.entity_a, c.entity_b))
        .collect();
    let recalled = injected.iter().filter(|pair| found.contains(pair)).count();
    let signals: usize = report.candidates.iter().map(|c| c.signals.len()).sum();
    let approx_bytes = report.candidates.len() * std::mem::size_of::<CandidateMatch>()
        + signals * (std::mem::size_of::<MatchSignal>() + 16);
    println!(
        "entities={} pairs_compared={} candidates={} injected={} recalled={} oversized_blocks={} \
         budget_reached={} duration_ms={:.2} candidate_memory≈{} KiB",
        report.entities_examined,
        report.pairs_compared,
        report.candidates.len(),
        injected.len(),
        recalled,
        report.oversized_blocks,
        report.comparison_budget_reached,
        elapsed.as_secs_f64() * 1000.0,
        approx_bytes / 1024
    );

    assert_eq!(report.entities_examined, base + duplicates);
    assert!(report.pairs_compared <= limits.max_comparisons);
    let all_pairs = (base + duplicates) * (base + duplicates - 1) / 2;
    assert!(
        report.pairs_compared < all_pairs / 10,
        "blocking must avoid the O(n²) comparison of all {all_pairs} pairs"
    );
    assert!(
        recalled as f64 >= injected.len() as f64 * 0.95,
        "recalled {recalled} of {} injected duplicates",
        injected.len()
    );
}

#[test]
fn four_hundred_customers_with_mixed_duplicates() {
    run(300, 100);
}

#[test]
fn the_largest_scan_stays_bounded() {
    // max_entities is at most 999.
    run(800, 199);
}
