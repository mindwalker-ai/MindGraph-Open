//! Logical identity groups and how approvals consolidate them.
//!
//! An approved "same entity" decision does not touch the graph. It records
//! that several physical entities represent one real-world identity, with
//! one of them chosen as the canonical entity:
//!
//! ```text
//! E1 (canonical) ◄── E2 (alias)
//!                ◄── E3 (alias)
//! ```
//!
//! Groups are flat: every member points directly at the canonical entity,
//! which points at itself. Flat groups cannot contain chains or cycles, and
//! resolving any member is one lookup. Approving a pair from two groups
//! consolidates them into one group with the canonical entity the reviewer
//! chose from the union (transitive consolidation):
//!
//! ```text
//! E2 → E1,  E4 → E3,  approve (E2, E4) with canonical E3
//!     ⇒  E1 → E3, E2 → E3, E4 → E3, E3 → E3
//! ```

use std::collections::BTreeSet;

use mindgraph_domain::EntityId;

use crate::MergeError;

/// The default largest identity group.
pub const DEFAULT_MAX_GROUP_MEMBERS: usize = 100;

/// One logical identity: a canonical entity and its members (the canonical
/// entity included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityGroup {
    canonical: EntityId,
    members: BTreeSet<EntityId>,
}

impl IdentityGroup {
    /// A group of `members` with `canonical` (added to the members if
    /// missing).
    #[must_use]
    pub fn new(canonical: EntityId, members: impl IntoIterator<Item = EntityId>) -> Self {
        let mut members: BTreeSet<EntityId> = members.into_iter().collect();
        members.insert(canonical);
        Self { canonical, members }
    }

    /// The canonical entity.
    #[must_use]
    pub const fn canonical(&self) -> EntityId {
        self.canonical
    }

    /// Every member, canonical included, in id order.
    #[must_use]
    pub const fn members(&self) -> &BTreeSet<EntityId> {
        &self.members
    }

    /// Whether `entity` belongs to the group.
    #[must_use]
    pub fn contains(&self, entity: EntityId) -> bool {
        self.members.contains(&entity)
    }

    /// The number of members.
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Always `false`: a group has at least its canonical entity.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

/// The metadata changes of one approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePlan {
    /// The canonical entity of the resulting group.
    pub canonical: EntityId,
    /// Every member of the resulting group.
    pub members: BTreeSet<EntityId>,
    /// The canonical entities of the groups before the approval.
    pub previous_canonicals: Vec<EntityId>,
    /// Members whose stored mapping must be written (new members, and
    /// members whose canonical entity changes). Empty when the approval
    /// changes nothing.
    pub changed: Vec<EntityId>,
}

impl MergePlan {
    /// Whether the approval changes no mapping.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.changed.is_empty()
    }
}

/// Plans the approval of `a` and `b` as the same identity with `canonical`.
///
/// `group_a` and `group_b` are the current groups of `a` and `b`, if any.
/// The canonical entity must be a member of the union. No chains or cycles
/// can result, because every member is pointed directly at `canonical`.
///
/// # Errors
///
/// [`MergeError::SelfPair`], [`MergeError::CanonicalOutsideGroup`],
/// [`MergeError::GroupTooLarge`], or [`MergeError::InconsistentGroup`] when
/// a supplied group does not contain its entity.
pub fn plan_merge(
    a: EntityId,
    b: EntityId,
    group_a: Option<&IdentityGroup>,
    group_b: Option<&IdentityGroup>,
    canonical: EntityId,
    max_members: usize,
) -> Result<MergePlan, MergeError> {
    if a == b {
        return Err(MergeError::SelfPair);
    }
    if group_a.is_some_and(|group| !group.contains(a))
        || group_b.is_some_and(|group| !group.contains(b))
    {
        return Err(MergeError::InconsistentGroup);
    }
    let mut members = BTreeSet::from([a, b]);
    let mut previous = BTreeSet::new();
    for group in [group_a, group_b].into_iter().flatten() {
        members.extend(group.members().iter().copied());
        previous.insert(group.canonical());
    }
    if !members.contains(&canonical) {
        return Err(MergeError::CanonicalOutsideGroup);
    }
    if members.len() > max_members {
        return Err(MergeError::GroupTooLarge { max: max_members });
    }
    let current = |entity: EntityId| {
        [group_a, group_b]
            .into_iter()
            .flatten()
            .find(|group| group.contains(entity))
            .map(IdentityGroup::canonical)
    };
    let changed = members
        .iter()
        .copied()
        .filter(|&member| current(member) != Some(canonical))
        .collect();
    Ok(MergePlan {
        canonical,
        members,
        previous_canonicals: previous.into_iter().collect(),
        changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<const N: usize>() -> [EntityId; N] {
        let mut ids = [EntityId::new(); N];
        for id in &mut ids {
            *id = EntityId::new();
        }
        ids
    }

    #[test]
    fn one_approved_pair_forms_a_group() {
        let [e1, e2] = ids();
        let plan = plan_merge(e1, e2, None, None, e1, 100).unwrap();
        assert_eq!(plan.canonical, e1);
        assert_eq!(plan.members, BTreeSet::from([e1, e2]));
        assert_eq!(plan.changed, [e1, e2], "both need a mapping row");
        assert!(plan.previous_canonicals.is_empty());
    }

    #[test]
    fn the_reviewer_chooses_the_canonical_entity() {
        let [e1, e2] = ids();
        // Not the oldest id: the choice is the reviewer's.
        let plan = plan_merge(e1, e2, None, None, e2, 100).unwrap();
        assert_eq!(plan.canonical, e2);
    }

    #[test]
    fn an_alias_joins_an_existing_group() {
        let [e1, e2, e3] = ids();
        let group = IdentityGroup::new(e1, [e2]);
        let plan = plan_merge(e1, e3, Some(&group), None, e1, 100).unwrap();
        assert_eq!(plan.members, BTreeSet::from([e1, e2, e3]));
        assert_eq!(plan.changed, [e3], "existing members keep their mapping");
    }

    #[test]
    fn two_groups_consolidate_transitively_under_one_canonical() {
        let [e1, e2, e3, e4] = ids();
        let left = IdentityGroup::new(e1, [e2]);
        let right = IdentityGroup::new(e3, [e4]);
        let plan = plan_merge(e2, e4, Some(&left), Some(&right), e3, 100).unwrap();
        assert_eq!(plan.canonical, e3);
        assert_eq!(plan.members, BTreeSet::from([e1, e2, e3, e4]));
        assert_eq!(
            plan.previous_canonicals,
            BTreeSet::from([e1, e3]).into_iter().collect::<Vec<_>>()
        );
        assert_eq!(plan.changed, [e1, e2], "only the left group moves");
    }

    #[test]
    fn approving_a_pair_already_in_one_group_changes_nothing() {
        let [e1, e2] = ids();
        let group = IdentityGroup::new(e1, [e2]);
        let plan = plan_merge(e1, e2, Some(&group), Some(&group), e1, 100).unwrap();
        assert!(plan.is_noop());
        // Choosing another member re-points the whole group.
        let plan = plan_merge(e1, e2, Some(&group), Some(&group), e2, 100).unwrap();
        assert_eq!(plan.changed, [e1, e2]);
    }

    #[test]
    fn invalid_merges_are_rejected() {
        let [e1, e2, outsider] = ids();
        assert_eq!(
            plan_merge(e1, e1, None, None, e1, 100),
            Err(MergeError::SelfPair)
        );
        assert_eq!(
            plan_merge(e1, e2, None, None, outsider, 100),
            Err(MergeError::CanonicalOutsideGroup)
        );
        assert_eq!(
            plan_merge(e1, e2, None, None, e1, 1),
            Err(MergeError::GroupTooLarge { max: 1 })
        );
        let unrelated = IdentityGroup::new(outsider, []);
        assert_eq!(
            plan_merge(e1, e2, Some(&unrelated), None, e1, 100),
            Err(MergeError::InconsistentGroup)
        );
    }

    #[test]
    fn groups_stay_flat_so_cycles_cannot_form() {
        let [e1, e2, e3] = ids();
        // E2 → E1, then approve (E1, E3) with canonical E3, then (E3, E2)
        // with canonical E2: every step keeps one canonical for everyone.
        let first = IdentityGroup::new(e1, [e2]);
        let plan = plan_merge(e1, e3, Some(&first), None, e3, 100).unwrap();
        let second = IdentityGroup::new(plan.canonical, plan.members.clone());
        let plan = plan_merge(e3, e2, Some(&second), Some(&second), e2, 100).unwrap();
        assert_eq!(plan.canonical, e2);
        assert_eq!(plan.members.len(), 3);
    }
}
