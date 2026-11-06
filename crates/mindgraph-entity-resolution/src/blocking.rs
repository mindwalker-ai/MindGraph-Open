//! Blocking: which pairs are worth comparing.
//!
//! Comparing every entity with every other one is O(n²). Instead, each rule
//! puts every entity with a usable value into one or more *blocks*, and only
//! pairs that share a block are compared:
//!
//! | Rule | Block key |
//! |---|---|
//! | `exact_identifier` | the trimmed identifier |
//! | `normalized_email` | the lower-cased address |
//! | `normalized_phone` | the E.164 number |
//! | `normalized_string` | the normalised text |
//! | `fuzzy_name` | the first three letters of each name token of two or more letters |
//!
//! A block larger than [`ScanLimits::max_block_size`](crate::ScanLimits) is
//! skipped and reported (a very common value such as a frequent first name
//! says little about identity), so the scan says it was not exhaustive
//! instead of silently comparing a huge group. Two names that share no token
//! prefix (a typo in the first letters of every token) are never compared:
//! a known limit of this conservative scheme.

use std::collections::{BTreeMap, BTreeSet};

use crate::matching::Key;

/// Letters of a name token used as its block key.
pub const NAME_BLOCK_PREFIX: usize = 3;

/// The block keys of one prepared value.
pub(crate) fn block_keys(key: &Key) -> Vec<String> {
    match key {
        Key::Identifier(value) => vec![value.clone()],
        Key::Email { key, .. } => vec![key.folded.clone()],
        Key::Phone { e164, .. } => vec![e164.clone()],
        Key::Text { normalized, .. } => vec![normalized.clone()],
        Key::Name { key, .. } => {
            let prefixes: BTreeSet<String> = key
                .tokens
                .iter()
                .filter(|token| token.chars().count() >= 2)
                .map(|token| token.chars().take(NAME_BLOCK_PREFIX).collect())
                .collect();
            prefixes.into_iter().collect()
        }
    }
}

/// The blocks of one rule: block key to member indices (ascending).
pub(crate) fn build_blocks<'a>(
    keys: impl Iterator<Item = (usize, Option<&'a Key>)>,
) -> BTreeMap<String, Vec<usize>> {
    let mut blocks: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, key) in keys {
        let Some(key) = key else { continue };
        for block in block_keys(key) {
            blocks.entry(block).or_default().push(index);
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Normalized, name_key};

    fn name(value: &str) -> Key {
        let Normalized::Value(key) = name_key(value) else {
            panic!()
        };
        Key::Name {
            raw: value.into(),
            key,
        }
    }

    #[test]
    fn names_block_on_token_prefixes_but_not_initials() {
        assert_eq!(block_keys(&name("M. Ikhwan Nasution")), ["ikh", "nas"]);
        assert_eq!(
            block_keys(&name("Muhammad Ikhwan Nasution")),
            ["ikh", "muh", "nas"]
        );
        assert_eq!(block_keys(&name("Al Tan")), ["al", "tan"]);
    }

    #[test]
    fn blocks_list_members_in_index_order() {
        let keys = [name("Alice Tan"), name("Budi Santoso"), name("A. Tan")];
        let blocks = build_blocks(keys.iter().enumerate().map(|(i, k)| (i, Some(k))));
        assert_eq!(blocks["tan"], [0, 2]);
        assert_eq!(blocks["san"], [1]);
    }
}
