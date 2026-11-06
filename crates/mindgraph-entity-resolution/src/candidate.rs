//! Candidate pairs and the evidence behind them.
//!
//! Evidence names properties, rule kinds and outcomes, plus a similarity for
//! fuzzy rules. It never contains property values, so it can be stored and
//! logged without copying personal data out of the graph. Reviewers see the
//! values by reading both entities from the GraphStore.

use mindgraph_domain::EntityId;
use serde::{Deserialize, Serialize};

use crate::RuleKind;

/// What one rule found for one candidate pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalOutcome {
    /// The values are identical as written (after trimming).
    Exact,
    /// The values are equal only after normalisation (case, whitespace,
    /// Unicode form, phone format).
    Normalized,
    /// The names are similar enough (at or above the rule's threshold) but
    /// not equal.
    Fuzzy,
    /// Both values are present and usable, and differ.
    NoMatch,
    /// At least one entity has no value. Missing data never matches.
    Missing,
    /// A value is present but cannot be compared, for example an invalid
    /// phone number. See the signal's note.
    Unavailable,
}

impl SignalOutcome {
    /// Whether this outcome counts as agreeing evidence.
    #[must_use]
    pub const fn is_match(self) -> bool {
        matches!(self, Self::Exact | Self::Normalized | Self::Fuzzy)
    }
}

/// Extra context for a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalNote {
    /// The name similarity relied on an initial (`M.` for `Muhammad`), which
    /// is ambiguous.
    InitialsMatched,
    /// The email addresses differ only in the letter case of their local
    /// part, which some mail servers treat as different mailboxes.
    CaseInsensitiveLocalPart,
    /// The value is not a valid email address or phone number.
    InvalidValue,
    /// A national-format phone number with no default country configured.
    MissingCountry,
    /// The phone number is stored as an integer, which may have lost leading
    /// zeros; it is not reconstructed.
    IntegerPhone,
    /// The value is too long to compare.
    ValueTooLong,
    /// The stored value does not have the type the rule compares.
    TypeMismatch,
}

impl SignalNote {
    /// The stable name of this note.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InitialsMatched => "initials_matched",
            Self::CaseInsensitiveLocalPart => "case_insensitive_local_part",
            Self::InvalidValue => "invalid_value",
            Self::MissingCountry => "missing_country",
            Self::IntegerPhone => "integer_phone",
            Self::ValueTooLong => "value_too_long",
            Self::TypeMismatch => "type_mismatch",
        }
    }
}

/// The evidence of one rule for one candidate pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchSignal {
    /// The compared property.
    pub property: String,
    /// The rule kind.
    pub rule: RuleKind,
    /// What the rule found.
    pub outcome: SignalOutcome,
    /// Whether the signal counts as agreeing evidence.
    pub matched: bool,
    /// The name similarity in `[0, 1]` (fuzzy rules with two usable values).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
    /// The signal's weight in the score; `0` when it did not match.
    pub weight: f64,
    /// Extra context, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<SignalNote>,
}

/// A potential duplicate: two entities of the same type with at least one
/// agreeing signal. Always a suggestion for human review, never a decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateMatch {
    /// The smaller entity id of the pair.
    pub entity_a: EntityId,
    /// The larger entity id of the pair.
    pub entity_b: EntityId,
    /// The ranking heuristic in `[0, 1)`, rounded to four decimals. It orders
    /// review work; it is not a probability that the entities are the same.
    pub score: f64,
    /// One signal per configured rule, in rule order.
    pub signals: Vec<MatchSignal>,
}

impl CandidateMatch {
    /// The rule kinds that agreed, in rule order.
    pub fn matched_rules(&self) -> impl Iterator<Item = &MatchSignal> {
        self.signals.iter().filter(|signal| signal.matched)
    }
}

/// Orders a pair so the smaller id comes first; `None` for a self-pair.
#[must_use]
pub fn ordered_pair(a: EntityId, b: EntityId) -> Option<(EntityId, EntityId)> {
    match a.cmp(&b) {
        std::cmp::Ordering::Less => Some((a, b)),
        std::cmp::Ordering::Greater => Some((b, a)),
        std::cmp::Ordering::Equal => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_are_ordered_and_self_pairs_rejected() {
        let (a, b) = (EntityId::new(), EntityId::new());
        assert_eq!(ordered_pair(b, a), Some((a, b)));
        assert_eq!(ordered_pair(a, b), Some((a, b)));
        assert_eq!(ordered_pair(a, a), None);
    }

    #[test]
    fn signals_serialise_without_values() {
        let signal = MatchSignal {
            property: "email".into(),
            rule: RuleKind::NormalizedEmail,
            outcome: SignalOutcome::Normalized,
            matched: true,
            similarity: None,
            weight: 0.75,
            note: Some(SignalNote::CaseInsensitiveLocalPart),
        };
        let json = serde_json::to_value(&signal).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "property": "email",
                "rule": "normalized_email",
                "outcome": "normalized",
                "matched": true,
                "weight": 0.75,
                "note": "case_insensitive_local_part"
            })
        );
        let back: MatchSignal = serde_json::from_value(json).unwrap();
        assert_eq!(back, signal);
    }
}
