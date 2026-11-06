//! The bounded candidate scan: normalise, block, compare, score.
//!
//! [`find_candidates`] is deterministic: the same entities and rules always
//! give the same candidates, scores and order. It is pure (no I/O) and
//! bounded by [`ScanLimits`]: at most `max_comparisons` distinct pairs are
//! compared, blocks larger than `max_block_size` are skipped, and at most
//! `max_candidates` candidates are returned. Every limit that cut the work
//! short is reported, so a caller never presents a partial scan as
//! exhaustive.
//!
//! Pairs are discovered rule by rule, strongest evidence first
//! (`exact_identifier`, email, phone, normalised string, fuzzy name), so a
//! comparison budget that runs out cuts weak fuzzy-only pairs first. Every
//! discovered pair is compared with *all* rules, so its evidence is complete.

use std::collections::{BTreeMap, HashSet};

use mindgraph_domain::{Entity, PropertyValue};
use serde::{Deserialize, Serialize};

use crate::blocking::build_blocks;
use crate::normalize::{
    Country, EmailKey, Equality, NameKey, Normalized, equality, name_key, normalize_email,
    normalize_phone, text_key,
};
use crate::scoring::{
    EMAIL_CASE_INSENSITIVE_WEIGHT, EMAIL_WEIGHT, EXACT_IDENTIFIER_WEIGHT, NAME_WEIGHT,
    PHONE_WEIGHT, STRING_WEIGHT, combine_weights, round4,
};
use crate::similarity::name_similarity;
use crate::{CandidateMatch, MatchRule, MatchSignal, RuleKind, RuleSet, SignalNote, SignalOutcome};

/// The bounds of one scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    /// Most distinct pairs compared.
    pub max_comparisons: usize,
    /// Largest block whose pairs are compared; larger blocks are skipped.
    pub max_block_size: usize,
    /// Most candidates returned (highest scores first).
    pub max_candidates: usize,
}

impl ScanLimits {
    /// The default comparison budget.
    pub const DEFAULT_MAX_COMPARISONS: usize = 20_000;
    /// The default largest block.
    pub const DEFAULT_MAX_BLOCK_SIZE: usize = 50;
    /// The default candidate cap.
    pub const DEFAULT_MAX_CANDIDATES: usize = 2_000;
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_comparisons: Self::DEFAULT_MAX_COMPARISONS,
            max_block_size: Self::DEFAULT_MAX_BLOCK_SIZE,
            max_candidates: Self::DEFAULT_MAX_CANDIDATES,
        }
    }
}

/// Why part of a scan's data could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    /// Blocks larger than the block limit were skipped.
    OversizedBlocks,
    /// Values were not valid emails or phone numbers.
    InvalidValues,
    /// National-format phone numbers with no default country.
    MissingCountry,
    /// Phone numbers stored as integers (leading zeros may be lost).
    IntegerPhone,
    /// Values too long to compare.
    ValuesTooLong,
    /// Values whose stored type differs from the rule's property type.
    TypeMismatch,
}

impl WarningCode {
    fn for_note(note: SignalNote) -> Option<Self> {
        match note {
            SignalNote::InvalidValue => Some(Self::InvalidValues),
            SignalNote::MissingCountry => Some(Self::MissingCountry),
            SignalNote::IntegerPhone => Some(Self::IntegerPhone),
            SignalNote::ValueTooLong => Some(Self::ValuesTooLong),
            SignalNote::TypeMismatch => Some(Self::TypeMismatch),
            SignalNote::InitialsMatched | SignalNote::CaseInsensitiveLocalPart => None,
        }
    }
}

/// A data-quality finding of a scan: counts and names, never values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanWarning {
    /// What happened.
    pub code: WarningCode,
    /// The rule's property.
    pub property: String,
    /// The rule kind.
    pub rule: RuleKind,
    /// How many values (or, for oversized blocks, blocks) were affected.
    pub count: usize,
}

/// The result of one scan.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanReport {
    /// Candidates, highest score first, then by entity ids.
    pub candidates: Vec<CandidateMatch>,
    /// Distinct entities of the rule set's type that were examined.
    pub entities_examined: usize,
    /// Distinct pairs compared.
    pub pairs_compared: usize,
    /// Whether the comparison budget stopped the scan early.
    pub comparison_budget_reached: bool,
    /// Blocks skipped for exceeding the block limit.
    pub oversized_blocks: usize,
    /// Whether candidates beyond `max_candidates` were dropped.
    pub candidate_limit_reached: bool,
    /// Data-quality findings, in rule order.
    pub warnings: Vec<ScanWarning>,
}

impl ScanReport {
    /// Whether every pair sharing a block was compared and kept. Note that a
    /// scan over a truncated entity subset is still not exhaustive for the
    /// graph; the caller knows about that truncation.
    #[must_use]
    pub const fn complete(&self) -> bool {
        !self.comparison_budget_reached
            && self.oversized_blocks == 0
            && !self.candidate_limit_reached
    }
}

/// One entity's prepared value for one rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key {
    Identifier(String),
    Email { raw: String, key: EmailKey },
    Phone { raw: String, e164: String },
    Text { raw: String, normalized: String },
    Name { raw: String, key: NameKey },
}

fn prepare(
    rule: &MatchRule,
    country: Option<Country>,
    value: Option<&PropertyValue>,
) -> Normalized<Key> {
    let Some(value) = value else {
        return Normalized::Missing;
    };
    match (rule.kind, value) {
        (RuleKind::ExactIdentifier, PropertyValue::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Normalized::Missing
            } else {
                Normalized::Value(Key::Identifier(trimmed.to_owned()))
            }
        }
        (RuleKind::ExactIdentifier, PropertyValue::Integer(number)) => {
            Normalized::Value(Key::Identifier(number.to_string()))
        }
        (RuleKind::NormalizedEmail, PropertyValue::String(text)) => match normalize_email(text) {
            Normalized::Value(key) => Normalized::Value(Key::Email {
                raw: text.trim().to_owned(),
                key,
            }),
            Normalized::Missing => Normalized::Missing,
            Normalized::Unusable(note) => Normalized::Unusable(note),
        },
        (RuleKind::NormalizedPhone, PropertyValue::String(text)) => {
            match normalize_phone(text, country) {
                Normalized::Value(e164) => Normalized::Value(Key::Phone {
                    raw: text.trim().to_owned(),
                    e164,
                }),
                Normalized::Missing => Normalized::Missing,
                Normalized::Unusable(note) => Normalized::Unusable(note),
            }
        }
        (RuleKind::NormalizedPhone, PropertyValue::Integer(_)) => {
            Normalized::Unusable(SignalNote::IntegerPhone)
        }
        (RuleKind::NormalizedString, PropertyValue::String(text)) => match text_key(text) {
            Normalized::Value(normalized) => Normalized::Value(Key::Text {
                raw: text.trim().to_owned(),
                normalized,
            }),
            Normalized::Missing => Normalized::Missing,
            Normalized::Unusable(note) => Normalized::Unusable(note),
        },
        (RuleKind::FuzzyName, PropertyValue::String(text)) => match name_key(text) {
            Normalized::Value(key) => Normalized::Value(Key::Name {
                raw: text.trim().to_owned(),
                key,
            }),
            Normalized::Missing => Normalized::Missing,
            Normalized::Unusable(note) => Normalized::Unusable(note),
        },
        _ => Normalized::Unusable(SignalNote::TypeMismatch),
    }
}

fn signal(rule: &MatchRule, outcome: SignalOutcome) -> MatchSignal {
    MatchSignal {
        property: rule.property.to_string(),
        rule: rule.kind,
        outcome,
        matched: false,
        similarity: None,
        weight: 0.0,
        note: None,
    }
}

fn matched(mut signal: MatchSignal, weight: f64) -> MatchSignal {
    signal.matched = true;
    signal.weight = round4(weight);
    signal
}

fn outcome_of(equality: Equality) -> SignalOutcome {
    match equality {
        Equality::Exact => SignalOutcome::Exact,
        Equality::Normalized => SignalOutcome::Normalized,
        Equality::Different => SignalOutcome::NoMatch,
    }
}

/// The evidence of one rule for one pair.
fn compare(rule: &MatchRule, a: &Normalized<Key>, b: &Normalized<Key>) -> MatchSignal {
    let (a, b) = match (a, b) {
        (Normalized::Value(a), Normalized::Value(b)) => (a, b),
        (Normalized::Unusable(note), _) | (_, Normalized::Unusable(note)) => {
            let mut signal = signal(rule, SignalOutcome::Unavailable);
            signal.note = Some(*note);
            return signal;
        }
        _ => return signal(rule, SignalOutcome::Missing),
    };
    match (a, b) {
        (Key::Identifier(x), Key::Identifier(y)) => {
            if x == y {
                matched(signal(rule, SignalOutcome::Exact), EXACT_IDENTIFIER_WEIGHT)
            } else {
                signal(rule, SignalOutcome::NoMatch)
            }
        }
        (Key::Email { raw: ra, key: ka }, Key::Email { raw: rb, key: kb }) => {
            match equality(ra, rb, &ka.exact, &kb.exact) {
                Equality::Different if rule.email_case_insensitive() && ka.folded == kb.folded => {
                    let mut signal = signal(rule, SignalOutcome::Normalized);
                    signal.note = Some(SignalNote::CaseInsensitiveLocalPart);
                    matched(signal, EMAIL_CASE_INSENSITIVE_WEIGHT)
                }
                Equality::Different => signal(rule, SignalOutcome::NoMatch),
                other => matched(signal(rule, outcome_of(other)), EMAIL_WEIGHT),
            }
        }
        (Key::Phone { raw: ra, e164: ea }, Key::Phone { raw: rb, e164: eb }) => {
            match equality(ra, rb, ea, eb) {
                Equality::Different => signal(rule, SignalOutcome::NoMatch),
                other => matched(signal(rule, outcome_of(other)), PHONE_WEIGHT),
            }
        }
        (
            Key::Text {
                raw: ra,
                normalized: na,
            },
            Key::Text {
                raw: rb,
                normalized: nb,
            },
        ) => match equality(ra, rb, na, nb) {
            Equality::Different => signal(rule, SignalOutcome::NoMatch),
            other => matched(signal(rule, outcome_of(other)), STRING_WEIGHT),
        },
        (Key::Name { raw: ra, key: ka }, Key::Name { raw: rb, key: kb }) => {
            let similarity = name_similarity(ka, kb);
            let mut base = signal(rule, SignalOutcome::NoMatch);
            base.similarity = Some(similarity.score);
            if ka.joined == kb.joined {
                base.outcome = if ra == rb {
                    SignalOutcome::Exact
                } else {
                    SignalOutcome::Normalized
                };
                matched(base, NAME_WEIGHT)
            } else if similarity.score >= rule.fuzzy_threshold() {
                base.outcome = SignalOutcome::Fuzzy;
                base.note = similarity
                    .used_initials
                    .then_some(SignalNote::InitialsMatched);
                matched(base, NAME_WEIGHT * similarity.score)
            } else {
                base
            }
        }
        // Keys of one rule always have the same shape.
        _ => signal(rule, SignalOutcome::Unavailable),
    }
}

/// Compares two entities with every rule; `None` unless at least one signal
/// agrees. The entities are returned in id order.
#[must_use]
pub fn compare_entities(rules: &RuleSet, a: &Entity, b: &Entity) -> Option<CandidateMatch> {
    let (first, second) = if a.id() < b.id() { (a, b) } else { (b, a) };
    if first.id() == second.id()
        || first.entity_type() != &rules.entity_type
        || second.entity_type() != &rules.entity_type
    {
        return None;
    }
    let prepared = |entity: &Entity| -> Vec<Normalized<Key>> {
        rules
            .rules
            .iter()
            .map(|rule| {
                prepare(
                    rule,
                    rule.country(),
                    entity.properties().get(rule.property.as_str()),
                )
            })
            .collect()
    };
    let (pa, pb) = (prepared(first), prepared(second));
    candidate(rules, first, second, &pa, &pb)
}

fn candidate(
    rules: &RuleSet,
    a: &Entity,
    b: &Entity,
    pa: &[Normalized<Key>],
    pb: &[Normalized<Key>],
) -> Option<CandidateMatch> {
    let signals: Vec<MatchSignal> = rules
        .rules
        .iter()
        .zip(pa.iter().zip(pb))
        .map(|(rule, (x, y))| compare(rule, x, y))
        .collect();
    if !signals.iter().any(|signal| signal.matched) {
        return None;
    }
    let score = combine_weights(signals.iter().filter(|s| s.matched).map(|s| s.weight));
    Some(CandidateMatch {
        entity_a: a.id(),
        entity_b: b.id(),
        score,
        signals,
    })
}

/// Finds potential duplicates among `entities` of `rules.entity_type` (see
/// the [module docs](self)). Entities of other types are ignored, so a
/// `Customer` is never compared with a `Merchant`.
#[must_use]
pub fn find_candidates(entities: &[Entity], rules: &RuleSet, limits: &ScanLimits) -> ScanReport {
    let mut entities: Vec<&Entity> = entities
        .iter()
        .filter(|entity| entity.entity_type() == &rules.entity_type)
        .collect();
    entities.sort_by_key(|entity| entity.id());
    entities.dedup_by_key(|entity| entity.id());

    let countries: Vec<Option<Country>> = rules.rules.iter().map(MatchRule::country).collect();
    let prepared: Vec<Vec<Normalized<Key>>> = entities
        .iter()
        .map(|entity| {
            rules
                .rules
                .iter()
                .zip(&countries)
                .map(|(rule, country)| {
                    prepare(
                        rule,
                        *country,
                        entity.properties().get(rule.property.as_str()),
                    )
                })
                .collect()
        })
        .collect();

    // Unusable values per (rule, warning code).
    let mut unusable: BTreeMap<(usize, WarningCode), usize> = BTreeMap::new();
    for values in &prepared {
        for (index, value) in values.iter().enumerate() {
            if let Normalized::Unusable(note) = value
                && let Some(code) = WarningCode::for_note(*note)
            {
                *unusable.entry((index, code)).or_default() += 1;
            }
        }
    }

    // Strongest rule kinds first; ties keep the configured order.
    let mut order: Vec<usize> = (0..rules.rules.len()).collect();
    order.sort_by_key(|&index| (rules.rules[index].kind, index));

    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut candidates = Vec::new();
    let mut pairs_compared = 0;
    let mut budget_reached = false;
    let mut oversized: BTreeMap<usize, usize> = BTreeMap::new();

    'rules: for &rule_index in &order {
        let blocks = build_blocks(
            prepared
                .iter()
                .enumerate()
                .map(|(entity, values)| (entity, values[rule_index].value())),
        );
        for members in blocks.values() {
            if members.len() < 2 {
                continue;
            }
            if members.len() > limits.max_block_size {
                *oversized.entry(rule_index).or_default() += 1;
                continue;
            }
            for (position, &i) in members.iter().enumerate() {
                for &j in &members[position + 1..] {
                    if !seen.insert((i, j)) {
                        continue;
                    }
                    if pairs_compared >= limits.max_comparisons {
                        budget_reached = true;
                        break 'rules;
                    }
                    pairs_compared += 1;
                    if let Some(found) =
                        candidate(rules, entities[i], entities[j], &prepared[i], &prepared[j])
                    {
                        candidates.push(found);
                    }
                }
            }
        }
    }

    candidates.sort_by(|x, y| {
        y.score
            .total_cmp(&x.score)
            .then(x.entity_a.cmp(&y.entity_a))
            .then(x.entity_b.cmp(&y.entity_b))
    });
    let candidate_limit_reached = candidates.len() > limits.max_candidates;
    candidates.truncate(limits.max_candidates);

    let mut warnings = Vec::new();
    for (index, rule) in rules.rules.iter().enumerate() {
        if let Some(&count) = oversized.get(&index) {
            warnings.push(ScanWarning {
                code: WarningCode::OversizedBlocks,
                property: rule.property.to_string(),
                rule: rule.kind,
                count,
            });
        }
        for ((_, code), &count) in unusable
            .iter()
            .filter(|((rule_index, _), _)| *rule_index == index)
        {
            warnings.push(ScanWarning {
                code: *code,
                property: rule.property.to_string(),
                rule: rule.kind,
                count,
            });
        }
    }

    ScanReport {
        candidates,
        entities_examined: entities.len(),
        pairs_compared,
        comparison_budget_reached: budget_reached,
        oversized_blocks: oversized.values().sum(),
        candidate_limit_reached,
        warnings,
    }
}
