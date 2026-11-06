//! Fuzzy name similarity.
//!
//! The similarity of two names is a **token-aware Jaro-Winkler** score
//! (`strsim`): tokens are paired one-to-one, greedily by descending
//! Jaro-Winkler similarity, and the sum of the paired scores is divided by
//! the token count of the *longer* name, so a missing middle name lowers the
//! score. Each token tolerates typos (`Santoso` / `Santosa`), and token order
//! does not matter (`Tan Alice` / `Alice Tan`). A one-letter token (an
//! initial) pairs with a token that starts with that letter at a fixed
//! [`INITIAL_SIMILARITY`]; such matches are flagged because initials are
//! ambiguous.
//!
//! Only when the two names have a different number of tokens is
//! Jaro-Winkler on the whole names also considered (and the larger score
//! kept), so joined or split names (`AliceTan` / `Alice Tan`) still compare
//! well. It is deliberately not used for names with the same token count: a
//! long shared surname would otherwise lift different first names
//! (`Dewi Santoso` / `Dian Santoso`, often relatives) over the threshold.
//!
//! The result is a heuristic in `[0, 1]`, not a calibrated probability.
//! Common names produce high similarity between different people, so a name
//! match alone is weak evidence and never approves an identity.

use crate::normalize::NameKey;
use crate::scoring::round4;

/// Similarity of an initial and a token starting with that letter.
pub const INITIAL_SIMILARITY: f64 = 0.85;

/// The similarity of two names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NameSimilarity {
    /// The similarity in `[0, 1]`, rounded to four decimals.
    pub score: f64,
    /// Whether the score relied on an initial.
    pub used_initials: bool,
}

/// The similarity of two tokens, and whether it relied on an initial.
fn token_similarity(a: &str, b: &str) -> (f64, bool) {
    if a == b {
        return (1.0, false);
    }
    let initial = |token: &str| {
        let mut chars = token.chars();
        chars.next().filter(|_| chars.next().is_none())
    };
    match (initial(a), initial(b)) {
        (Some(letter), None) => (
            if b.starts_with(letter) {
                INITIAL_SIMILARITY
            } else {
                0.0
            },
            b.starts_with(letter),
        ),
        (None, Some(letter)) => (
            if a.starts_with(letter) {
                INITIAL_SIMILARITY
            } else {
                0.0
            },
            a.starts_with(letter),
        ),
        (Some(_), Some(_)) => (0.0, false),
        (None, None) => (strsim::jaro_winkler(a, b), false),
    }
}

/// Greedy one-to-one token pairing, divided by the longer name's length.
fn token_score(a: &[String], b: &[String]) -> (f64, bool) {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if long.is_empty() {
        return (0.0, false);
    }
    let mut pairs = Vec::with_capacity(short.len() * long.len());
    for (i, x) in short.iter().enumerate() {
        for (j, y) in long.iter().enumerate() {
            let (score, initial) = token_similarity(x, y);
            pairs.push((score, i, j, initial));
        }
    }
    // Highest score first; ties by position, so the result is deterministic.
    pairs.sort_by(|p, q| q.0.total_cmp(&p.0).then(p.1.cmp(&q.1)).then(p.2.cmp(&q.2)));
    let mut used_short = vec![false; short.len()];
    let mut used_long = vec![false; long.len()];
    let mut sum = 0.0;
    let mut initials = false;
    for (score, i, j, initial) in pairs {
        if used_short[i] || used_long[j] || score <= 0.0 {
            continue;
        }
        used_short[i] = true;
        used_long[j] = true;
        sum += score;
        initials |= initial;
    }
    #[allow(clippy::cast_precision_loss)] // token counts are tiny
    let score = sum / long.len() as f64;
    (score, initials)
}

/// The fuzzy similarity of two names (see the [module docs](self)).
///
/// ```
/// use mindgraph_entity_resolution::{Normalized, name_key, name_similarity};
///
/// let key = |s| match name_key(s) { Normalized::Value(k) => k, _ => unreachable!() };
/// let s = name_similarity(&key("Muhammad Ikhwan Nasution"), &key("M. Ikhwan Nasution"));
/// assert!(s.score >= 0.9 && s.used_initials);
/// ```
#[must_use]
pub fn name_similarity(a: &NameKey, b: &NameKey) -> NameSimilarity {
    if a.joined == b.joined {
        return NameSimilarity {
            score: 1.0,
            used_initials: false,
        };
    }
    let (tokens, initials) = token_score(&a.tokens, &b.tokens);
    let whole = if a.tokens.len() == b.tokens.len() {
        0.0
    } else {
        strsim::jaro_winkler(&a.joined, &b.joined)
    };
    if tokens >= whole {
        NameSimilarity {
            score: round4(tokens),
            used_initials: initials,
        }
    } else {
        NameSimilarity {
            score: round4(whole),
            used_initials: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Normalized, name_key};

    fn similarity(a: &str, b: &str) -> NameSimilarity {
        let key = |s: &str| match name_key(s) {
            Normalized::Value(key) => key,
            other => panic!("{other:?}"),
        };
        name_similarity(&key(a), &key(b))
    }

    #[test]
    fn identical_names_after_normalisation_are_one() {
        assert_eq!(similarity("Alice  TAN", "alice tan").score, 1.0);
    }

    #[test]
    fn initials_score_high_but_are_flagged() {
        let s = similarity("Muhammad Ikhwan Nasution", "M. Ikhwan Nasution");
        assert_eq!(s.score, 0.95);
        assert!(s.used_initials);
        // An initial that does not fit pairs with nothing.
        assert!(similarity("Budi Santoso", "A. Santoso").score < 0.9);
    }

    #[test]
    fn typos_and_token_order_are_tolerated() {
        assert!(similarity("Budi Santoso", "Budi Santosa").score > 0.95);
        assert_eq!(similarity("Tan Alice", "Alice Tan").score, 1.0);
    }

    #[test]
    fn a_missing_token_lowers_the_score() {
        let partial = similarity("Ikhwan Nasution", "Muhammad Ikhwan Nasution").score;
        assert!(partial < 0.9, "{partial}");
        let full = similarity("Muhammad Ikhwan Nasution", "Muhammad Ikhwan Nasution").score;
        assert!(partial < full);
    }

    #[test]
    fn a_shared_surname_does_not_make_different_first_names_similar() {
        // Often relatives, not one person.
        assert!(similarity("Dewi Santoso", "Dian Santoso").score < 0.9);
        assert!(similarity("Agus Wijaya", "Bayu Wijaya").score < 0.9);
    }

    #[test]
    fn joined_and_split_names_still_compare_well() {
        assert!(similarity("AliceTan", "Alice Tan").score >= 0.9);
    }

    #[test]
    fn different_names_score_low() {
        assert!(similarity("Alice Tan", "Budi Santoso").score < 0.7);
        assert!(similarity("Citra Lestari", "Eko Prasetyo").score < 0.7);
    }

    #[test]
    fn similarity_is_symmetric_and_deterministic() {
        for (a, b) in [
            ("M. Ikhwan K. Nasution", "Muhammad Ikhwan Nasution"),
            ("Dewi Kusuma", "Dewi Kusumawati"),
            ("A B C", "C B A D"),
        ] {
            assert_eq!(similarity(a, b), similarity(b, a), "{a} / {b}");
            assert_eq!(similarity(a, b), similarity(a, b));
        }
    }
}
