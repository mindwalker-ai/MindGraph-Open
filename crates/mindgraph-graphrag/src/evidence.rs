//! The evidence graph of an answer and its citable references.
//!
//! An [`EvidenceGraph`] holds only entities and relationships read from the
//! GraphStore for one graph, deduplicated by MindGraph id (parallel
//! relationships keep their own ids), with every relationship's endpoints
//! present and every path made of consecutive, connected items. Nothing is
//! ever synthesised to fill a gap.
//!
//! References are positional and stable for one evidence graph: `E1` is the
//! first entity, `R1` the first relationship, `P1` the first path. The model
//! may cite them; [`EvidenceGraph::resolve_citations`] keeps only references
//! that exist. A valid citation proves that the cited graph object was
//! retrieved, not that the model's reading of it is right.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;

use mindgraph_domain::{Entity, EntityId, GraphId, Relationship, RelationshipId};

use crate::EvidenceError;
use crate::limits::GraphRagLimits;

/// An ordered path through the evidence graph: `relationship_ids[i]`
/// connects `entity_ids[i]` and `entity_ids[i + 1]` (in either direction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidencePath {
    entity_ids: Vec<EntityId>,
    relationship_ids: Vec<RelationshipId>,
}

impl EvidencePath {
    /// A path; checked against the evidence graph when added.
    #[must_use]
    pub fn new(entity_ids: Vec<EntityId>, relationship_ids: Vec<RelationshipId>) -> Self {
        Self {
            entity_ids,
            relationship_ids,
        }
    }

    /// The entities in order.
    #[must_use]
    pub fn entity_ids(&self) -> &[EntityId] {
        &self.entity_ids
    }

    /// The relationships in order.
    #[must_use]
    pub fn relationship_ids(&self) -> &[RelationshipId] {
        &self.relationship_ids
    }
}

/// Members connected through one shared entity (shared-neighbour results).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedGroup {
    /// The shared entity, for example a device.
    pub shared: EntityId,
    /// The members, for example the customers using it, ordered by id.
    pub members: Vec<EntityId>,
}

/// The authoritative evidence of one answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EvidenceGraph {
    entities: Vec<Entity>,
    relationships: Vec<Relationship>,
    paths: Vec<EvidencePath>,
    groups: Vec<SharedGroup>,
}

/// The kind of a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RefKind {
    /// `E…`
    Entity,
    /// `R…`
    Relationship,
    /// `P…`
    Path,
}

impl RefKind {
    /// `entity`, `relationship` or `path`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Entity => "entity",
            Self::Relationship => "relationship",
            Self::Path => "path",
        }
    }

    const fn prefix(self) -> char {
        match self {
            Self::Entity => 'E',
            Self::Relationship => 'R',
            Self::Path => 'P',
        }
    }
}

/// A reference such as `E3`: a kind and a 1-based position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EvidenceRef {
    /// The kind.
    pub kind: RefKind,
    /// The 1-based position.
    pub number: usize,
}

impl EvidenceRef {
    const fn new(kind: RefKind, index: usize) -> Self {
        Self {
            kind,
            number: index + 1,
        }
    }
}

impl fmt::Display for EvidenceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.kind.prefix(), self.number)
    }
}

impl FromStr for EvidenceRef {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.trim();
        let mut chars = raw.chars();
        let kind = match chars.next() {
            Some('E') => RefKind::Entity,
            Some('R') => RefKind::Relationship,
            Some('P') => RefKind::Path,
            _ => return Err(()),
        };
        let digits = chars.as_str();
        if digits.is_empty()
            || digits.len() > 6
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || digits.starts_with('0')
        {
            return Err(());
        }
        let number = digits.parse().map_err(|_| ())?;
        Ok(Self { kind, number })
    }
}

/// What a valid citation points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CitedObject {
    /// An evidence entity.
    Entity(EntityId),
    /// An evidence relationship.
    Relationship(RelationshipId),
    /// An evidence path (its 0-based position).
    Path(usize),
}

/// A citation that refers to retrieved evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Citation {
    /// The reference, for example `E1`.
    pub reference: EvidenceRef,
    /// The cited object.
    pub object: CitedObject,
}

/// The outcome of checking citations against an evidence graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CitationCheck {
    /// Citations that exist, deduplicated, in the order first given.
    pub citations: Vec<Citation>,
    /// How many cited references do not exist in the evidence graph.
    pub unknown: usize,
}

impl EvidenceGraph {
    /// The entities, in reference order (`E1` first).
    #[must_use]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// The relationships, in reference order (`R1` first).
    #[must_use]
    pub fn relationships(&self) -> &[Relationship] {
        &self.relationships
    }

    /// The paths, in reference order (`P1` first).
    #[must_use]
    pub fn paths(&self) -> &[EvidencePath] {
        &self.paths
    }

    /// The shared-neighbour groups.
    #[must_use]
    pub fn groups(&self) -> &[SharedGroup] {
        &self.groups
    }

    /// Whether the graph holds no entity.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// The reference of an entity, for example `E2`.
    #[must_use]
    pub fn entity_ref(&self, id: EntityId) -> Option<EvidenceRef> {
        self.entities
            .iter()
            .position(|entity| entity.id() == id)
            .map(|index| EvidenceRef::new(RefKind::Entity, index))
    }

    /// The reference of a relationship, for example `R1`.
    #[must_use]
    pub fn relationship_ref(&self, id: RelationshipId) -> Option<EvidenceRef> {
        self.relationships
            .iter()
            .position(|relationship| relationship.id() == id)
            .map(|index| EvidenceRef::new(RefKind::Relationship, index))
    }

    /// The reference of the path at `index`.
    #[must_use]
    pub fn path_ref(&self, index: usize) -> Option<EvidenceRef> {
        (index < self.paths.len()).then(|| EvidenceRef::new(RefKind::Path, index))
    }

    /// The object a reference points to, if it exists.
    #[must_use]
    pub fn resolve(&self, reference: EvidenceRef) -> Option<CitedObject> {
        let index = reference.number.checked_sub(1)?;
        match reference.kind {
            RefKind::Entity => self
                .entities
                .get(index)
                .map(|e| CitedObject::Entity(e.id())),
            RefKind::Relationship => self
                .relationships
                .get(index)
                .map(|r| CitedObject::Relationship(r.id())),
            RefKind::Path => (index < self.paths.len()).then_some(CitedObject::Path(index)),
        }
    }

    /// Keeps the cited references that exist in this evidence graph. Unknown
    /// or malformed references are counted, never kept: a fabricated id can
    /// never become a citation.
    #[must_use]
    pub fn resolve_citations<S: AsRef<str>>(&self, cited: &[S]) -> CitationCheck {
        let mut seen = HashSet::new();
        let mut check = CitationCheck::default();
        for raw in cited {
            match raw
                .as_ref()
                .parse::<EvidenceRef>()
                .ok()
                .and_then(|reference| {
                    self.resolve(reference)
                        .map(|object| Citation { reference, object })
                }) {
                Some(citation) => {
                    if seen.insert(citation.reference) {
                        check.citations.push(citation);
                    }
                }
                None => check.unknown += 1,
            }
        }
        check
    }

    /// Checks every invariant: one graph, unique ids, endpoints present,
    /// paths connected.
    ///
    /// # Errors
    ///
    /// The first broken invariant.
    pub fn validate(&self, graph_id: GraphId) -> Result<(), EvidenceError> {
        let mut entities = HashSet::new();
        for entity in &self.entities {
            let id = entity.id().to_string();
            if entity.graph_id() != graph_id {
                return Err(EvidenceError::ForeignGraph { kind: "entity", id });
            }
            if !entities.insert(entity.id()) {
                return Err(EvidenceError::Duplicate { kind: "entity", id });
            }
        }
        let mut relationships = HashMap::new();
        for relationship in &self.relationships {
            let id = relationship.id().to_string();
            if relationship.graph_id() != graph_id {
                return Err(EvidenceError::ForeignGraph {
                    kind: "relationship",
                    id,
                });
            }
            if relationships
                .insert(relationship.id(), relationship)
                .is_some()
            {
                return Err(EvidenceError::Duplicate {
                    kind: "relationship",
                    id,
                });
            }
            if !entities.contains(&relationship.source_id())
                || !entities.contains(&relationship.target_id())
            {
                return Err(EvidenceError::MissingEndpoint(relationship.id()));
            }
        }
        for path in &self.paths {
            check_path(path, &entities, &relationships)?;
        }
        for group in &self.groups {
            for id in std::iter::once(&group.shared).chain(&group.members) {
                if !entities.contains(id) {
                    return Err(EvidenceError::UnknownPathItem(id.to_string()));
                }
            }
        }
        Ok(())
    }

    /// A copy keeping only the given entities and relationships (in their
    /// original order), and the paths and groups made only of kept items.
    /// Relationships whose endpoints are dropped are dropped too.
    #[must_use]
    pub fn retain(
        &self,
        entities: &HashSet<EntityId>,
        relationships: &HashSet<RelationshipId>,
    ) -> Self {
        let kept_entities: Vec<Entity> = self
            .entities
            .iter()
            .filter(|entity| entities.contains(&entity.id()))
            .cloned()
            .collect();
        let present: HashSet<EntityId> = kept_entities.iter().map(Entity::id).collect();
        let kept_relationships: Vec<Relationship> = self
            .relationships
            .iter()
            .filter(|r| {
                relationships.contains(&r.id())
                    && present.contains(&r.source_id())
                    && present.contains(&r.target_id())
            })
            .cloned()
            .collect();
        let linked: HashSet<RelationshipId> =
            kept_relationships.iter().map(Relationship::id).collect();
        Self {
            paths: self
                .paths
                .iter()
                .filter(|path| {
                    path.entity_ids.iter().all(|id| present.contains(id))
                        && path.relationship_ids.iter().all(|id| linked.contains(id))
                })
                .cloned()
                .collect(),
            groups: self
                .groups
                .iter()
                .filter_map(|group| {
                    let members: Vec<EntityId> = group
                        .members
                        .iter()
                        .copied()
                        .filter(|id| present.contains(id))
                        .collect();
                    (present.contains(&group.shared) && members.len() >= 2).then_some(SharedGroup {
                        shared: group.shared,
                        members,
                    })
                })
                .collect(),
            entities: kept_entities,
            relationships: kept_relationships,
        }
    }
}

fn check_path(
    path: &EvidencePath,
    entities: &HashSet<EntityId>,
    relationships: &HashMap<RelationshipId, &Relationship>,
) -> Result<(), EvidenceError> {
    if path.entity_ids.len() != path.relationship_ids.len() + 1 {
        return Err(EvidenceError::BrokenPath);
    }
    for id in &path.entity_ids {
        if !entities.contains(id) {
            return Err(EvidenceError::UnknownPathItem(id.to_string()));
        }
    }
    for (pair, id) in path.entity_ids.windows(2).zip(&path.relationship_ids) {
        let relationship = relationships
            .get(id)
            .ok_or_else(|| EvidenceError::UnknownPathItem(id.to_string()))?;
        let (source, target) = (relationship.source_id(), relationship.target_id());
        if !((source == pair[0] && target == pair[1]) || (source == pair[1] && target == pair[0])) {
            return Err(EvidenceError::BrokenPath);
        }
    }
    Ok(())
}

/// Builds an evidence graph within the evidence budget. Additions are
/// all-or-nothing: a relationship is only added together with its
/// endpoints, so the budget never leaves a dangling relationship.
#[derive(Debug)]
pub struct EvidenceBuilder {
    graph_id: GraphId,
    max_entities: usize,
    max_relationships: usize,
    graph: EvidenceGraph,
    entity_ids: HashSet<EntityId>,
    relationship_ids: HashMap<RelationshipId, usize>,
    full: bool,
}

impl EvidenceBuilder {
    /// An empty builder for `graph_id` with the evidence budget of `limits`.
    #[must_use]
    pub fn new(graph_id: GraphId, limits: &GraphRagLimits) -> Self {
        Self {
            graph_id,
            max_entities: limits.max_evidence_entities,
            max_relationships: limits.max_evidence_relationships,
            graph: EvidenceGraph::default(),
            entity_ids: HashSet::new(),
            relationship_ids: HashMap::new(),
            full: false,
        }
    }

    /// Whether something was left out because the budget was full.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.full
    }

    /// Whether the entity is already in the evidence graph.
    #[must_use]
    pub fn contains_entity(&self, id: EntityId) -> bool {
        self.entity_ids.contains(&id)
    }

    /// Room left: (entities, relationships).
    #[must_use]
    pub fn remaining(&self) -> (usize, usize) {
        (
            self.max_entities.saturating_sub(self.graph.entities.len()),
            self.max_relationships
                .saturating_sub(self.graph.relationships.len()),
        )
    }

    fn check_entity(&self, entity: &Entity) -> Result<(), EvidenceError> {
        if entity.graph_id() == self.graph_id {
            Ok(())
        } else {
            Err(EvidenceError::ForeignGraph {
                kind: "entity",
                id: entity.id().to_string(),
            })
        }
    }

    /// Adds entities and relationships together, or nothing if they do not
    /// all fit (the builder is then marked full). Already present items are
    /// skipped; every relationship's endpoints must be among `entities` or
    /// already present.
    ///
    /// # Errors
    ///
    /// An item of another graph, or a relationship with an endpoint that is
    /// neither given nor present.
    pub fn add_all(
        &mut self,
        entities: &[&Entity],
        relationships: &[&Relationship],
    ) -> Result<bool, EvidenceError> {
        let mut new_entities: Vec<&Entity> = Vec::new();
        let mut planned: HashSet<EntityId> = HashSet::new();
        for entity in entities {
            self.check_entity(entity)?;
            if !self.entity_ids.contains(&entity.id()) && planned.insert(entity.id()) {
                new_entities.push(entity);
            }
        }
        let mut new_relationships: Vec<&Relationship> = Vec::new();
        let mut planned_relationships = HashSet::new();
        for relationship in relationships {
            if relationship.graph_id() != self.graph_id {
                return Err(EvidenceError::ForeignGraph {
                    kind: "relationship",
                    id: relationship.id().to_string(),
                });
            }
            for endpoint in [relationship.source_id(), relationship.target_id()] {
                if !self.entity_ids.contains(&endpoint) && !planned.contains(&endpoint) {
                    return Err(EvidenceError::MissingEndpoint(relationship.id()));
                }
            }
            if !self.relationship_ids.contains_key(&relationship.id())
                && planned_relationships.insert(relationship.id())
            {
                new_relationships.push(relationship);
            }
        }
        let (entity_room, relationship_room) = self.remaining();
        if new_entities.len() > entity_room || new_relationships.len() > relationship_room {
            self.full = true;
            return Ok(false);
        }
        for entity in new_entities {
            self.entity_ids.insert(entity.id());
            self.graph.entities.push(entity.clone());
        }
        for relationship in new_relationships {
            self.relationship_ids
                .insert(relationship.id(), self.graph.relationships.len());
            self.graph.relationships.push(relationship.clone());
        }
        Ok(true)
    }

    /// Adds one entity (see [`Self::add_all`]).
    ///
    /// # Errors
    ///
    /// An entity of another graph.
    pub fn add_entity(&mut self, entity: &Entity) -> Result<bool, EvidenceError> {
        self.add_all(&[entity], &[])
    }

    /// Adds a path whose items are already present.
    ///
    /// # Errors
    ///
    /// A path naming absent items or not connecting its entities.
    pub fn add_path(&mut self, path: EvidencePath) -> Result<(), EvidenceError> {
        let relationships: HashMap<RelationshipId, &Relationship> = self
            .relationship_ids
            .iter()
            .map(|(id, index)| (*id, &self.graph.relationships[*index]))
            .collect();
        check_path(&path, &self.entity_ids, &relationships)?;
        if !self.graph.paths.contains(&path) {
            self.graph.paths.push(path);
        }
        Ok(())
    }

    /// Records a shared-neighbour group whose entities are present.
    ///
    /// # Errors
    ///
    /// A group naming absent entities.
    pub fn add_group(&mut self, group: SharedGroup) -> Result<(), EvidenceError> {
        for id in std::iter::once(&group.shared).chain(&group.members) {
            if !self.entity_ids.contains(id) {
                return Err(EvidenceError::UnknownPathItem(id.to_string()));
            }
        }
        self.graph.groups.push(group);
        Ok(())
    }

    /// The finished evidence graph.
    #[must_use]
    pub fn finish(self) -> EvidenceGraph {
        self.graph
    }
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::{Properties, TypeName};

    use super::*;

    fn entity(graph_id: GraphId, entity_type: &str) -> Entity {
        Entity::new(
            EntityId::new(),
            graph_id,
            TypeName::new(entity_type).unwrap(),
            Properties::new(),
        )
    }

    fn link(source: &Entity, target: &Entity) -> Relationship {
        Relationship::new(
            RelationshipId::new(),
            source.graph_id(),
            TypeName::new("USES").unwrap(),
            source.id(),
            target.id(),
            Properties::new(),
        )
    }

    fn limits(entities: usize, relationships: usize) -> GraphRagLimits {
        GraphRagLimits {
            max_evidence_entities: entities,
            max_evidence_relationships: relationships,
            ..GraphRagLimits::default()
        }
    }

    #[test]
    fn duplicates_are_removed_and_parallel_relationships_kept() {
        let graph_id = GraphId::new();
        let (a, b) = (entity(graph_id, "Customer"), entity(graph_id, "Device"));
        let (first, second) = (link(&a, &b), link(&a, &b));
        let mut builder = EvidenceBuilder::new(graph_id, &GraphRagLimits::default());
        assert!(
            builder
                .add_all(&[&a, &b, &a], &[&first, &second, &first])
                .unwrap()
        );
        assert!(builder.add_all(&[&b], &[&second]).unwrap());
        builder
            .add_path(EvidencePath::new(vec![a.id(), b.id()], vec![second.id()]))
            .unwrap();
        let graph = builder.finish();
        assert_eq!(graph.entities().len(), 2);
        assert_eq!(
            graph.relationships().len(),
            2,
            "parallel relationships stay"
        );
        assert_eq!(graph.entity_ref(b.id()).unwrap().to_string(), "E2");
        assert_eq!(
            graph.relationship_ref(second.id()).unwrap().to_string(),
            "R2"
        );
        graph.validate(graph_id).unwrap();
    }

    #[test]
    fn invariants_are_enforced() {
        let graph_id = GraphId::new();
        let (a, b) = (entity(graph_id, "Customer"), entity(graph_id, "Device"));
        let other = entity(GraphId::new(), "Customer");
        let mut builder = EvidenceBuilder::new(graph_id, &GraphRagLimits::default());
        assert!(matches!(
            builder.add_entity(&other),
            Err(EvidenceError::ForeignGraph { .. })
        ));
        let relationship = link(&a, &b);
        assert!(matches!(
            builder.add_all(&[&a], &[&relationship]),
            Err(EvidenceError::MissingEndpoint(_))
        ));
        builder.add_all(&[&a, &b], &[&relationship]).unwrap();
        let c = entity(graph_id, "Customer");
        builder.add_entity(&c).unwrap();
        assert!(matches!(
            builder.add_path(EvidencePath::new(
                vec![a.id(), c.id()],
                vec![relationship.id()]
            )),
            Err(EvidenceError::BrokenPath)
        ));
        assert!(matches!(
            builder.add_path(EvidencePath::new(
                vec![a.id(), b.id()],
                vec![RelationshipId::new()]
            )),
            Err(EvidenceError::UnknownPathItem(_))
        ));
    }

    #[test]
    fn the_budget_never_leaves_a_dangling_relationship() {
        let graph_id = GraphId::new();
        let (a, b, c) = (
            entity(graph_id, "Customer"),
            entity(graph_id, "Device"),
            entity(graph_id, "Customer"),
        );
        let mut builder = EvidenceBuilder::new(graph_id, &limits(2, 5));
        assert!(builder.add_all(&[&a, &b], &[&link(&a, &b)]).unwrap());
        assert!(!builder.add_all(&[&c], &[&link(&c, &b)]).unwrap());
        assert!(builder.is_full());
        let graph = builder.finish();
        assert_eq!(
            (graph.entities().len(), graph.relationships().len()),
            (2, 1)
        );
        graph.validate(graph_id).unwrap();
    }

    #[test]
    fn only_existing_references_become_citations() {
        let graph_id = GraphId::new();
        let (a, b) = (entity(graph_id, "Customer"), entity(graph_id, "Device"));
        let relationship = link(&a, &b);
        let mut builder = EvidenceBuilder::new(graph_id, &GraphRagLimits::default());
        builder.add_all(&[&a, &b], &[&relationship]).unwrap();
        builder
            .add_path(EvidencePath::new(
                vec![a.id(), b.id()],
                vec![relationship.id()],
            ))
            .unwrap();
        let graph = builder.finish();
        let check = graph.resolve_citations(&[
            "E1",
            "R1",
            "P1",
            "E1",
            "R9",
            "E0",
            "X1",
            "E01",
            relationship.id().to_string().as_str(),
        ]);
        assert_eq!(
            check
                .citations
                .iter()
                .map(|c| c.reference.to_string())
                .collect::<Vec<_>>(),
            ["E1", "R1", "P1"]
        );
        assert_eq!(check.unknown, 5);
        assert_eq!(
            check.citations[1].object,
            CitedObject::Relationship(relationship.id())
        );
    }

    #[test]
    fn retain_drops_dependent_relationships_and_paths() {
        let graph_id = GraphId::new();
        let (a, b, c) = (
            entity(graph_id, "Customer"),
            entity(graph_id, "Device"),
            entity(graph_id, "Customer"),
        );
        let (ab, cb) = (link(&a, &b), link(&c, &b));
        let mut builder = EvidenceBuilder::new(graph_id, &GraphRagLimits::default());
        builder.add_all(&[&a, &b, &c], &[&ab, &cb]).unwrap();
        builder
            .add_path(EvidencePath::new(
                vec![a.id(), b.id(), c.id()],
                vec![ab.id(), cb.id()],
            ))
            .unwrap();
        let graph = builder.finish();
        let kept = graph.retain(
            &[a.id(), b.id()].into_iter().collect(),
            &[ab.id(), cb.id()].into_iter().collect(),
        );
        assert_eq!(kept.relationships().len(), 1, "cb lost its endpoint");
        assert!(kept.paths().is_empty(), "the path depended on c");
        kept.validate(graph_id).unwrap();
    }
}
