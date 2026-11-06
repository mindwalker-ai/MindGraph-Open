//! Candidate scoring: an explainable ranking heuristic.
//!
//! Each agreeing signal has a fixed weight in `(0, 1)`:
//!
//! | Signal | Weight |
//! |---|---|
//! | `exact_identifier` | 0.95 |
//! | `normalized_email` | 0.90 (0.75 when only the local part's case differs) |
//! | `normalized_phone` | 0.85 |
//! | `normalized_string` | 0.60 |
//! | `fuzzy_name` | 0.60 × similarity |
//!
//! The score combines the weights of the agreeing signals as
//! `1 − Π (1 − weight)`: every agreeing signal raises it, it stays below 1,
//! and the same input always gives the same score (rounded to four
//! decimals). Disagreeing, missing and unusable signals do not lower it;
//! they are shown as evidence for the reviewer instead.
//!
//! The score orders review work. It is **not** a probability that two
//! entities are the same: `0.94` does not mean "94 % likely".

/// Weight of an equal configured identifier.
pub const EXACT_IDENTIFIER_WEIGHT: f64 = 0.95;
/// Weight of equal normalised email addresses.
pub const EMAIL_WEIGHT: f64 = 0.90;
/// Weight of email addresses that differ only in the local part's case.
pub const EMAIL_CASE_INSENSITIVE_WEIGHT: f64 = 0.75;
/// Weight of equal E.164 phone numbers.
pub const PHONE_WEIGHT: f64 = 0.85;
/// Weight of equal normalised strings.
pub const STRING_WEIGHT: f64 = 0.60;
/// Weight of a fuzzy name match, multiplied by its similarity.
pub const NAME_WEIGHT: f64 = 0.60;

/// Rounds to four decimals, so scores are stable across platforms and
/// compare exactly in storage.
#[must_use]
pub fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// Combines the weights of agreeing signals: `1 − Π (1 − weight)`.
///
/// ```
/// use mindgraph_entity_resolution::combine_weights;
///
/// assert_eq!(combine_weights([]), 0.0);
/// assert_eq!(combine_weights([0.9]), 0.9);
/// assert_eq!(combine_weights([0.9, 0.85]), 0.985);
/// ```
#[must_use]
pub fn combine_weights(weights: impl IntoIterator<Item = f64>) -> f64 {
    let remaining = weights
        .into_iter()
        .fold(1.0, |rest, weight| rest * (1.0 - weight.clamp(0.0, 1.0)));
    round4(1.0 - remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn more_agreeing_evidence_scores_higher_but_below_one() {
        let name_only = combine_weights([NAME_WEIGHT * 0.95]);
        let name_and_phone = combine_weights([NAME_WEIGHT * 0.95, PHONE_WEIGHT]);
        let all = combine_weights([
            EXACT_IDENTIFIER_WEIGHT,
            EMAIL_WEIGHT,
            PHONE_WEIGHT,
            NAME_WEIGHT,
        ]);
        assert!(name_only < name_and_phone && name_and_phone < all);
        assert!(all < 1.0);
    }

    #[test]
    fn order_does_not_change_the_score() {
        assert_eq!(
            combine_weights([EMAIL_WEIGHT, PHONE_WEIGHT, 0.57]),
            combine_weights([0.57, PHONE_WEIGHT, EMAIL_WEIGHT])
        );
    }

    #[test]
    fn rounding_is_four_decimals() {
        assert_eq!(round4(0.123_456), 0.1235);
        assert_eq!(round4(0.999_96), 1.0);
    }
}
