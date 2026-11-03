//! Query and result value objects for [`GraphStore`](crate::GraphStore).
//!
//! Only what M2 needs: typed search, neighbourhood expansion and shortest
//! path, each with an explicit upper bound so no call can scan or traverse
//! an unbounded part of a graph.

use std::collections::BTreeMap;

use mindgraph_domain::{Entity, EntityId, PropertyKey, PropertyValue, Relationship, TypeName};

use crate::GraphStoreError;

/// The maximum number of entity ids accepted by
/// [`GraphStore::get_subgraph`](crate::GraphStore::get_subgraph).
pub const MAX_SUBGRAPH_ENTITIES: usize = 1_000;

fn check_range(what: &str, value: usize, max: usize) -> Result<usize, GraphStoreError> {
    if (1..=max).contains(&value) {
        Ok(value)
    } else {
        Err(GraphStoreError::InvalidInput(format!(
            "{what} must be between 1 and {max}, got {value}"
        )))
    }
}

/// Which relationships of an entity to follow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Relationships that start at the entity (`entity ──> other`).
    Outgoing,
    /// Relationships that end at the entity (`other ──> entity`).
    Incoming,
    /// Both outgoing and incoming relationships.
    #[default]
    Both,
}

impl Direction {
    /// The entity reached by following `relationship` from `from` in this
    /// direction, or `None` if the relationship cannot be followed that way.
    pub(crate) fn follow(self, relationship: &Relationship, from: EntityId) -> Option<EntityId> {
        let (source, target) = (relationship.source_id(), relationship.target_id());
        match self {
            Self::Outgoing => (source == from).then_some(target),
            Self::Incoming => (target == from).then_some(source),
            Self::Both if source == from => Some(target),
            Self::Both => (target == from).then_some(source),
        }
    }
}

/// Finds entities by type and exact property values.
///
/// Results are ordered by [`EntityId`] (which is creation-time order for
/// UUIDv7 ids) and capped at [`EntitySearch::limit`]. There is deliberately
/// no full-text, fuzzy or semantic matching in v0.1.
///
/// ```
/// use mindgraph_domain::{PropertyKey, TypeName};
/// use mindgraph_graph::EntitySearch;
///
/// let search = EntitySearch::new()
///     .with_entity_type(TypeName::new("Customer").unwrap())
///     .with_property(PropertyKey::new("country").unwrap(), "Indonesia")
///     .with_limit(50)
///     .unwrap();
/// assert_eq!(search.limit(), 50);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySearch {
    entity_type: Option<TypeName>,
    property_filters: BTreeMap<PropertyKey, PropertyValue>,
    alternatives: Option<Vec<BTreeMap<PropertyKey, PropertyValue>>>,
    limit: usize,
}

impl EntitySearch {
    /// The limit used when none is set.
    pub const DEFAULT_LIMIT: usize = 100;
    /// The largest accepted limit.
    pub const MAX_LIMIT: usize = 1_000;
    /// The largest number of alternatives in [`Self::with_any_of`].
    pub const MAX_ALTERNATIVES: usize = 1_000;

    /// Matches every entity of the graph, up to [`Self::DEFAULT_LIMIT`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            entity_type: None,
            property_filters: BTreeMap::new(),
            alternatives: None,
            limit: Self::DEFAULT_LIMIT,
        }
    }

    /// Only matches entities of this type.
    #[must_use]
    pub fn with_entity_type(mut self, entity_type: TypeName) -> Self {
        self.entity_type = Some(entity_type);
        self
    }

    /// Only matches entities whose property `key` equals `value` exactly
    /// (same type and same value). Several filters must all match.
    #[must_use]
    pub fn with_property(mut self, key: PropertyKey, value: impl Into<PropertyValue>) -> Self {
        self.property_filters.insert(key, value.into());
        self
    }

    /// Only matches entities that equal at least one of `alternatives`
    /// exactly. Each alternative is a set of property values that must all
    /// match, for example one identity in a batch of identity lookups
    /// (`customer_id = C1`, or `customer_id = C2`, ...). Combined with the
    /// other filters by AND; replaces earlier alternatives.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStoreError::InvalidInput`] for no alternatives, more
    /// than [`Self::MAX_ALTERNATIVES`], or an empty alternative.
    pub fn with_any_of(
        mut self,
        alternatives: Vec<BTreeMap<PropertyKey, PropertyValue>>,
    ) -> Result<Self, GraphStoreError> {
        check_range(
            "number of alternatives",
            alternatives.len(),
            Self::MAX_ALTERNATIVES,
        )?;
        if alternatives.iter().any(BTreeMap::is_empty) {
            return Err(GraphStoreError::InvalidInput(
                "every alternative needs at least one property".to_owned(),
            ));
        }
        self.alternatives = Some(alternatives);
        Ok(self)
    }

    /// Sets the maximum number of results.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStoreError::InvalidInput`] unless
    /// `1 <= limit <= MAX_LIMIT`.
    pub fn with_limit(mut self, limit: usize) -> Result<Self, GraphStoreError> {
        self.limit = check_range("search limit", limit, Self::MAX_LIMIT)?;
        Ok(self)
    }

    /// The entity type filter, if any.
    #[must_use]
    pub fn entity_type(&self) -> Option<&TypeName> {
        self.entity_type.as_ref()
    }

    /// The exact property filters, in key order.
    #[must_use]
    pub fn property_filters(&self) -> &BTreeMap<PropertyKey, PropertyValue> {
        &self.property_filters
    }

    /// The alternatives of [`Self::with_any_of`], if set.
    #[must_use]
    pub fn alternatives(&self) -> Option<&[BTreeMap<PropertyKey, PropertyValue>]> {
        self.alternatives.as_deref()
    }

    /// The maximum number of results.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Whether `entity` satisfies the type, property and alternative filters.
    pub(crate) fn matches(&self, entity: &Entity) -> bool {
        let all = |filters: &BTreeMap<PropertyKey, PropertyValue>| {
            filters
                .iter()
                .all(|(key, value)| entity.properties().get(key.as_str()) == Some(value))
        };
        self.entity_type
            .as_ref()
            .is_none_or(|entity_type| entity.entity_type() == entity_type)
            && all(&self.property_filters)
            && self
                .alternatives
                .as_ref()
                .is_none_or(|alternatives| alternatives.iter().any(all))
    }
}

impl Default for EntitySearch {
    fn default() -> Self {
        Self::new()
    }
}

/// Expands the neighbourhood of one entity.
///
/// The limit applies to relationships: the matching relationships with the
/// smallest [`RelationshipId`](mindgraph_domain::RelationshipId)s are
/// returned, together with the entities at their ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighborQuery {
    direction: Direction,
    relationship_type: Option<TypeName>,
    limit: usize,
}

impl NeighborQuery {
    /// The limit used when none is set.
    pub const DEFAULT_LIMIT: usize = 100;
    /// The largest accepted limit.
    pub const MAX_LIMIT: usize = 1_000;

    /// Follows relationships in `direction`, up to [`Self::DEFAULT_LIMIT`].
    #[must_use]
    pub const fn new(direction: Direction) -> Self {
        Self {
            direction,
            relationship_type: None,
            limit: Self::DEFAULT_LIMIT,
        }
    }

    /// Only follows relationships of this type.
    #[must_use]
    pub fn with_relationship_type(mut self, relationship_type: TypeName) -> Self {
        self.relationship_type = Some(relationship_type);
        self
    }

    /// Sets the maximum number of relationships returned.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStoreError::InvalidInput`] unless
    /// `1 <= limit <= MAX_LIMIT`.
    pub fn with_limit(mut self, limit: usize) -> Result<Self, GraphStoreError> {
        self.limit = check_range("neighbor limit", limit, Self::MAX_LIMIT)?;
        Ok(self)
    }

    /// The direction to follow.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// The relationship type filter, if any.
    #[must_use]
    pub fn relationship_type(&self) -> Option<&TypeName> {
        self.relationship_type.as_ref()
    }

    /// The maximum number of relationships returned.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Whether `relationship` passes the type filter.
    pub(crate) fn accepts(&self, relationship: &Relationship) -> bool {
        self.relationship_type
            .as_ref()
            .is_none_or(|relationship_type| relationship.relationship_type() == relationship_type)
    }
}

impl Default for NeighborQuery {
    fn default() -> Self {
        Self::new(Direction::Both)
    }
}

/// Options for shortest-path search.
///
/// `max_depth` is the maximum number of relationships (hops) in the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathQuery {
    direction: Direction,
    max_depth: usize,
}

impl PathQuery {
    /// The depth used when none is set.
    pub const DEFAULT_MAX_DEPTH: usize = 3;
    /// The largest accepted depth.
    pub const MAX_DEPTH: usize = 6;

    /// Follows relationships in `direction`, up to
    /// [`Self::DEFAULT_MAX_DEPTH`] hops.
    #[must_use]
    pub const fn new(direction: Direction) -> Self {
        Self {
            direction,
            max_depth: Self::DEFAULT_MAX_DEPTH,
        }
    }

    /// Sets the maximum number of hops.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStoreError::InvalidInput`] unless
    /// `1 <= max_depth <= MAX_DEPTH`.
    pub fn with_max_depth(mut self, max_depth: usize) -> Result<Self, GraphStoreError> {
        self.max_depth = check_range("path max_depth", max_depth, Self::MAX_DEPTH)?;
        Ok(self)
    }

    /// The direction to follow.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// The maximum number of hops.
    #[must_use]
    pub const fn max_depth(&self) -> usize {
        self.max_depth
    }
}

impl Default for PathQuery {
    fn default() -> Self {
        Self::new(Direction::Both)
    }
}

/// A set of entities and relationships, ready to render.
///
/// Entities are ordered by [`EntityId`] and relationships by
/// [`RelationshipId`](mindgraph_domain::RelationshipId), without duplicates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Subgraph {
    entities: Vec<Entity>,
    relationships: Vec<Relationship>,
}

impl Subgraph {
    /// Builds a subgraph, sorting both lists by id and dropping duplicates
    /// (the first occurrence of an id wins).
    #[must_use]
    pub fn new(mut entities: Vec<Entity>, mut relationships: Vec<Relationship>) -> Self {
        entities.sort_by_key(Entity::id);
        entities.dedup_by_key(|entity| entity.id());
        relationships.sort_by_key(Relationship::id);
        relationships.dedup_by_key(|relationship| relationship.id());
        Self {
            entities,
            relationships,
        }
    }

    /// The entities, ordered by id.
    #[must_use]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// The relationships, ordered by id.
    #[must_use]
    pub fn relationships(&self) -> &[Relationship] {
        &self.relationships
    }

    /// Returns `true` if the subgraph has no entities.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Splits the subgraph into its entities and relationships.
    #[must_use]
    pub fn into_parts(self) -> (Vec<Entity>, Vec<Relationship>) {
        (self.entities, self.relationships)
    }
}

/// A path through the graph:
/// `entity₀ ─relationship₀─ entity₁ ─relationship₁─ … entityₙ`.
///
/// `relationships[i]` connects `entities[i]` and `entities[i + 1]` (in either
/// orientation, depending on the direction that was searched). A path from
/// an entity to itself has one entity and no relationships.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphPath {
    entities: Vec<Entity>,
    relationships: Vec<Relationship>,
}

impl GraphPath {
    /// Builds a path, checking that it is well formed.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStoreError::InvalidInput`] if there are no entities, if
    /// there is not exactly one relationship fewer than entities, or if a
    /// relationship does not connect its neighbouring entities.
    pub fn new(
        entities: Vec<Entity>,
        relationships: Vec<Relationship>,
    ) -> Result<Self, GraphStoreError> {
        if entities.len() != relationships.len() + 1 {
            return Err(GraphStoreError::InvalidInput(format!(
                "a path with {} relationship(s) needs {} entities, got {}",
                relationships.len(),
                relationships.len() + 1,
                entities.len()
            )));
        }
        for (pair, relationship) in entities.windows(2).zip(&relationships) {
            let (a, b) = (pair[0].id(), pair[1].id());
            let (source, target) = (relationship.source_id(), relationship.target_id());
            if !((source == a && target == b) || (source == b && target == a)) {
                return Err(GraphStoreError::InvalidInput(format!(
                    "relationship {} does not connect path entities {a} and {b}",
                    relationship.id()
                )));
            }
        }
        Ok(Self {
            entities,
            relationships,
        })
    }

    /// The entities in path order, from source to target.
    #[must_use]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// The relationships in path order.
    #[must_use]
    pub fn relationships(&self) -> &[Relationship] {
        &self.relationships
    }

    /// The number of hops (relationships) in the path.
    #[must_use]
    pub fn hops(&self) -> usize {
        self.relationships.len()
    }
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::{GraphId, Properties, RelationshipId};

    use super::*;

    fn entity(graph_id: GraphId) -> Entity {
        Entity::new(
            EntityId::new(),
            graph_id,
            TypeName::new("Node").unwrap(),
            Properties::new(),
        )
    }

    fn relationship(source: &Entity, target: &Entity) -> Relationship {
        Relationship::new(
            RelationshipId::new(),
            source.graph_id(),
            TypeName::new("LINKS").unwrap(),
            source.id(),
            target.id(),
            Properties::new(),
        )
    }

    #[test]
    fn limits_are_bounded() {
        assert!(EntitySearch::new().with_limit(0).is_err());
        assert!(EntitySearch::new().with_limit(1_001).is_err());
        assert_eq!(
            EntitySearch::new().with_limit(1_000).unwrap().limit(),
            1_000
        );
        assert!(NeighborQuery::default().with_limit(0).is_err());
        assert!(NeighborQuery::default().with_limit(1_001).is_err());
        assert!(matches!(
            PathQuery::default().with_max_depth(7),
            Err(GraphStoreError::InvalidInput(_))
        ));
        assert!(PathQuery::default().with_max_depth(0).is_err());
        assert_eq!(
            PathQuery::default().with_max_depth(6).unwrap().max_depth(),
            6
        );
    }

    #[test]
    fn defaults_are_sane() {
        assert_eq!(EntitySearch::default().limit(), EntitySearch::DEFAULT_LIMIT);
        assert_eq!(NeighborQuery::default().direction(), Direction::Both);
        assert_eq!(
            PathQuery::default().max_depth(),
            PathQuery::DEFAULT_MAX_DEPTH
        );
    }

    #[test]
    fn direction_follows_relationships() {
        let graph_id = GraphId::new();
        let (a, b) = (entity(graph_id), entity(graph_id));
        let ab = relationship(&a, &b);
        assert_eq!(Direction::Outgoing.follow(&ab, a.id()), Some(b.id()));
        assert_eq!(Direction::Outgoing.follow(&ab, b.id()), None);
        assert_eq!(Direction::Incoming.follow(&ab, b.id()), Some(a.id()));
        assert_eq!(Direction::Incoming.follow(&ab, a.id()), None);
        assert_eq!(Direction::Both.follow(&ab, a.id()), Some(b.id()));
        assert_eq!(Direction::Both.follow(&ab, b.id()), Some(a.id()));
    }

    #[test]
    fn subgraph_sorts_and_deduplicates() {
        let graph_id = GraphId::new();
        let (a, b) = (entity(graph_id), entity(graph_id));
        let ab = relationship(&a, &b);
        let subgraph = Subgraph::new(
            vec![b.clone(), a.clone(), b.clone()],
            vec![ab.clone(), ab.clone()],
        );
        assert_eq!(subgraph.entities(), &[a, b]);
        assert_eq!(subgraph.relationships(), &[ab]);
    }

    #[test]
    fn graph_path_validates_its_shape() {
        let graph_id = GraphId::new();
        let (a, b, c) = (entity(graph_id), entity(graph_id), entity(graph_id));
        let ab = relationship(&a, &b);
        let cb = relationship(&c, &b);

        let path =
            GraphPath::new(vec![a.clone(), b.clone(), c.clone()], vec![ab.clone(), cb]).unwrap();
        assert_eq!(path.hops(), 2);

        assert!(GraphPath::new(vec![a.clone()], vec![]).is_ok(), "self path");
        assert!(GraphPath::new(vec![], vec![]).is_err());
        assert!(GraphPath::new(vec![a.clone(), b.clone()], vec![]).is_err());
        assert!(
            GraphPath::new(vec![a, c], vec![ab]).is_err(),
            "relationship must connect its neighbours"
        );
    }
}
