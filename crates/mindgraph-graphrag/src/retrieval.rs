//! Bounded GraphStore retrieval for a validated plan.
//!
//! Every operation uses the existing GraphStore queries (exact search,
//! neighbourhoods, shortest path) with explicit limits, counts its calls
//! against the operation budget and stops with a truncation indicator rather
//! than reading more. Inputs are processed in id order, so the same data
//! always yields the same evidence. Nothing here writes to the graph.
//!
//! The limitations returned with an outcome are user-visible sentences.
//! They may quote the values a question matched on, so they belong in the
//! response and the model context, never in logs.

use std::collections::{BTreeMap, HashMap, HashSet};

use mindgraph_domain::{Entity, EntityId, GraphId, Relationship, RelationshipId, TypeName};
use mindgraph_graph::{Direction, EntitySearch, GraphStore, NeighborQuery, PathQuery, Subgraph};

use crate::RetrievalError;
use crate::evidence::{EvidenceBuilder, EvidenceGraph, EvidencePath, SharedGroup};
use crate::limits::{GraphRagLimits, MAX_AMBIGUOUS_CANDIDATES};
use crate::plan::{Anchor, RetrievalPlan, direction_name};

/// Counters of one retrieval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetrievalStats {
    /// GraphStore calls made.
    pub operations: usize,
    /// Candidate entities inspected (shared-neighbour candidates, lookup
    /// matches, expanded entities).
    pub candidates_inspected: usize,
    /// Whether a limit stopped the retrieval before the requested scope was
    /// fully read. When set, the result must not be presented as complete.
    pub truncated: bool,
}

/// Whether the entities a plan names could be identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Every anchor resolved; the evidence answers the plan (possibly with
    /// no matches).
    Resolved,
    /// An anchor matched no entity.
    NotFound,
    /// An anchor that must name one entity matched several.
    Ambiguous,
    /// No path exists within the requested depth (which proves nothing
    /// about longer paths).
    NoPath,
}

/// The result of a retrieval.
#[derive(Debug, Clone, PartialEq)]
pub struct RetrievalOutcome {
    /// The evidence graph.
    pub evidence: EvidenceGraph,
    /// Counters.
    pub stats: RetrievalStats,
    /// Whether the anchors resolved.
    pub resolution: Resolution,
    /// User-visible limitations of this result (bounded reads, ambiguity,
    /// missing entities), without duplicates.
    pub limitations: Vec<String>,
}

const fn reverse(direction: Direction) -> Direction {
    match direction {
        Direction::Outgoing => Direction::Incoming,
        Direction::Incoming => Direction::Outgoing,
        Direction::Both => Direction::Both,
    }
}

/// The entity at the other end of `relationship`, seen from `from`.
fn other_end(relationship: &Relationship, from: EntityId) -> EntityId {
    if relationship.source_id() == from {
        relationship.target_id()
    } else {
        relationship.source_id()
    }
}

enum Single {
    Found(Entity),
    Missing,
    Ambiguous,
}

struct Retriever<'a> {
    store: &'a dyn GraphStore,
    graph_id: GraphId,
    limits: GraphRagLimits,
    stats: RetrievalStats,
    out_of_operations: bool,
    limitations: Vec<String>,
    evidence: EvidenceBuilder,
}

impl<'a> Retriever<'a> {
    fn note(&mut self, text: String) {
        if !self.limitations.contains(&text) {
            self.limitations.push(text);
        }
    }

    fn truncate(&mut self, text: String) {
        self.stats.truncated = true;
        self.note(text);
    }

    fn budget_full(&mut self) {
        let text = format!(
            "The evidence budget ({} entities, {} relationships) was reached; more results may exist.",
            self.limits.max_evidence_entities, self.limits.max_evidence_relationships
        );
        self.truncate(text);
    }

    /// Takes one operation from the budget, or reports that it is spent.
    fn operation(&mut self) -> bool {
        if self.stats.operations >= self.limits.max_operations {
            if !self.out_of_operations {
                self.out_of_operations = true;
                let text = format!(
                    "Retrieval stopped after {} graph operations (the per-question budget); more results may exist.",
                    self.limits.max_operations
                );
                self.truncate(text);
            }
            return false;
        }
        self.stats.operations += 1;
        true
    }

    async fn get_entity(&mut self, id: EntityId) -> Result<Option<Entity>, RetrievalError> {
        if !self.operation() {
            return Ok(None);
        }
        Ok(self.store.get_entity(self.graph_id, id).await?)
    }

    async fn search(
        &mut self,
        search: &EntitySearch,
    ) -> Result<Option<Vec<Entity>>, RetrievalError> {
        if !self.operation() {
            return Ok(None);
        }
        Ok(Some(
            self.store.search_entities(self.graph_id, search).await?,
        ))
    }

    async fn neighbors(
        &mut self,
        id: EntityId,
        direction: Direction,
        relationship_type: Option<&TypeName>,
    ) -> Result<Option<Subgraph>, RetrievalError> {
        if !self.operation() {
            return Ok(None);
        }
        let mut query =
            NeighborQuery::new(direction).with_limit(self.limits.max_neighbors_per_call)?;
        if let Some(relationship_type) = relationship_type {
            query = query.with_relationship_type(relationship_type.clone());
        }
        let subgraph = self.store.get_neighbors(self.graph_id, id, &query).await?;
        if subgraph.relationships().len() >= self.limits.max_neighbors_per_call {
            let text = format!(
                "Some entities have more than {} matching relationships; only the first {} of each were read.",
                self.limits.max_neighbors_per_call, self.limits.max_neighbors_per_call
            );
            self.truncate(text);
        }
        Ok(Some(subgraph))
    }

    fn match_search(
        entity_type: &TypeName,
        filters: &BTreeMap<mindgraph_domain::PropertyKey, mindgraph_domain::PropertyValue>,
        limit: usize,
    ) -> Result<EntitySearch, RetrievalError> {
        let mut search = EntitySearch::new()
            .with_entity_type(entity_type.clone())
            .with_limit(limit)?;
        for (key, value) in filters {
            search = search.with_property(key.clone(), value.clone());
        }
        Ok(search)
    }

    /// Resolves an anchor that must name exactly one entity.
    async fn single(&mut self, anchor: &Anchor) -> Result<Single, RetrievalError> {
        match anchor {
            Anchor::Entity { id, .. } => match self.get_entity(*id).await? {
                Some(entity) => Ok(Single::Found(entity)),
                None => {
                    self.note(format!("The {anchor} was not found in this graph."));
                    Ok(Single::Missing)
                }
            },
            Anchor::Match {
                entity_type,
                filters,
            } => {
                let search =
                    Self::match_search(entity_type, filters, MAX_AMBIGUOUS_CANDIDATES + 1)?;
                let Some(mut found) = self.search(&search).await? else {
                    return Ok(Single::Missing);
                };
                match found.len() {
                    0 => {
                        self.note(format!("No {anchor} was found (exact match)."));
                        Ok(Single::Missing)
                    }
                    1 => Ok(Single::Found(found.remove(0))),
                    count => {
                        let shown = count.min(MAX_AMBIGUOUS_CANDIDATES);
                        let many = if count > MAX_AMBIGUOUS_CANDIDATES {
                            format!("more than {MAX_AMBIGUOUS_CANDIDATES}")
                        } else {
                            count.to_string()
                        };
                        for candidate in found.iter().take(shown) {
                            if !self.evidence.add_entity(candidate)? {
                                break;
                            }
                        }
                        self.note(format!(
                            "The {anchor} matched {many} entities, so the question is ambiguous. Select one of them and ask again."
                        ));
                        Ok(Single::Ambiguous)
                    }
                }
            }
        }
    }

    fn unresolved(single: &Single) -> Resolution {
        match single {
            Single::Ambiguous => Resolution::Ambiguous,
            _ => Resolution::NotFound,
        }
    }

    async fn entity_lookup(&mut self, target: &Anchor) -> Result<Resolution, RetrievalError> {
        if let Anchor::Entity { .. } = target {
            return match self.single(target).await? {
                Single::Found(entity) => {
                    self.stats.candidates_inspected += 1;
                    self.evidence.add_entity(&entity)?;
                    Ok(Resolution::Resolved)
                }
                other => Ok(Self::unresolved(&other)),
            };
        }
        let Anchor::Match {
            entity_type,
            filters,
        } = target
        else {
            return Ok(Resolution::NotFound);
        };
        let limit = self.limits.max_candidates;
        let search = Self::match_search(entity_type, filters, limit)?;
        let Some(found) = self.search(&search).await? else {
            return Ok(Resolution::NotFound);
        };
        if found.is_empty() {
            self.note(format!("No {target} was found (exact match)."));
            return Ok(Resolution::NotFound);
        }
        self.stats.candidates_inspected += found.len();
        for entity in &found {
            if !self.evidence.add_entity(entity)? {
                self.budget_full();
                break;
            }
        }
        if found.len() >= limit {
            self.truncate(format!(
                "Only the first {limit} {target} (ordered by id) were read; more may exist."
            ));
        }
        Ok(Resolution::Resolved)
    }

    async fn neighbors_of(
        &mut self,
        center: &Anchor,
        direction: Direction,
        relationship_type: Option<&TypeName>,
        entity_type: Option<&TypeName>,
        depth: usize,
    ) -> Result<Resolution, RetrievalError> {
        let center = match self.single(center).await? {
            Single::Found(entity) => entity,
            other => return Ok(Self::unresolved(&other)),
        };
        let center_id = center.id();
        let mut entities: HashMap<EntityId, Entity> = HashMap::from([(center_id, center)]);
        let mut parent: HashMap<EntityId, EntityId> = HashMap::new();
        let mut recorded: Vec<Relationship> = Vec::new();
        let mut recorded_ids: HashSet<RelationshipId> = HashSet::new();
        let mut frontier = vec![center_id];
        'levels: for _ in 0..depth {
            let mut next = Vec::new();
            frontier.sort();
            for id in &frontier {
                let Some(subgraph) = self.neighbors(*id, direction, relationship_type).await?
                else {
                    break 'levels;
                };
                self.stats.candidates_inspected += 1;
                let (found, relationships) = subgraph.into_parts();
                for entity in found {
                    entities.entry(entity.id()).or_insert(entity);
                }
                for relationship in relationships {
                    let other = other_end(&relationship, *id);
                    if other != center_id
                        && !parent.contains_key(&other)
                        && entities.contains_key(&other)
                    {
                        parent.insert(other, *id);
                        next.push(other);
                    }
                    if recorded_ids.insert(relationship.id()) {
                        recorded.push(relationship);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        // Which entities to report: all reached ones, or only those of the
        // requested type plus the entities that connect them to the centre.
        let mut keep: HashSet<EntityId> = HashSet::from([center_id]);
        let reached: Vec<EntityId> = parent.keys().copied().collect();
        for id in reached {
            if entity_type.is_some_and(|wanted| entities[&id].entity_type() != wanted) {
                continue;
            }
            let mut current = id;
            while keep.insert(current) {
                match parent.get(&current) {
                    Some(up) => current = *up,
                    None => break,
                }
            }
        }
        // The centre first, then relationships in retrieval order with their
        // endpoints: each addition is all-or-nothing within the budget.
        self.evidence.add_entity(&entities[&center_id])?;
        for relationship in &recorded {
            let (source, target) = (relationship.source_id(), relationship.target_id());
            if !keep.contains(&source) || !keep.contains(&target) {
                continue;
            }
            if !self
                .evidence
                .add_all(&[&entities[&source], &entities[&target]], &[relationship])?
            {
                self.budget_full();
                break;
            }
        }
        Ok(Resolution::Resolved)
    }

    async fn find_path(
        &mut self,
        source: &Anchor,
        target: &Anchor,
        direction: Direction,
        max_depth: usize,
    ) -> Result<Resolution, RetrievalError> {
        let source_entity = match self.single(source).await? {
            Single::Found(entity) => entity,
            other => return Ok(Self::unresolved(&other)),
        };
        let target_entity = match self.single(target).await? {
            Single::Found(entity) => entity,
            other => return Ok(Self::unresolved(&other)),
        };
        if !self.operation() {
            return Ok(Resolution::NoPath);
        }
        let query = PathQuery::new(direction).with_max_depth(max_depth)?;
        let path = self
            .store
            .find_path(
                self.graph_id,
                source_entity.id(),
                target_entity.id(),
                &query,
            )
            .await?;
        let Some(path) = path else {
            self.evidence
                .add_all(&[&source_entity, &target_entity], &[])?;
            let following = match direction {
                Direction::Both => String::new(),
                other => format!(" following {} relationships", direction_name(other)),
            };
            self.note(format!(
                "No path was found within {max_depth} hops{following}. A longer path may still exist."
            ));
            return Ok(Resolution::NoPath);
        };
        let entities: Vec<&Entity> = path.entities().iter().collect();
        let relationships: Vec<&Relationship> = path.relationships().iter().collect();
        if !self.evidence.add_all(&entities, &relationships)? {
            self.budget_full();
            return Ok(Resolution::Resolved);
        }
        self.evidence.add_path(EvidencePath::new(
            path.entities().iter().map(Entity::id).collect(),
            path.relationships().iter().map(Relationship::id).collect(),
        ))?;
        Ok(Resolution::Resolved)
    }

    #[allow(clippy::too_many_lines)]
    async fn shared_neighbor(
        &mut self,
        start: Option<&Anchor>,
        member_type: Option<&TypeName>,
        shared_type: &TypeName,
        relationship_type: Option<&TypeName>,
        direction: Direction,
    ) -> Result<Resolution, RetrievalError> {
        let mut member_type = member_type.cloned();
        let mut fallback: Option<(Entity, Subgraph)> = None;
        let candidates: Vec<Entity> = if let Some(anchor) = start {
            let start = match self.single(anchor).await? {
                Single::Found(entity) => entity,
                other => return Ok(Self::unresolved(&other)),
            };
            if start.entity_type() == shared_type {
                vec![start]
            } else if member_type
                .as_ref()
                .is_none_or(|member| member == start.entity_type())
            {
                member_type = Some(start.entity_type().clone());
                let Some(subgraph) = self
                    .neighbors(start.id(), direction, relationship_type)
                    .await?
                else {
                    return Ok(Resolution::Resolved);
                };
                let mut candidates: Vec<Entity> = subgraph
                    .entities()
                    .iter()
                    .filter(|entity| {
                        entity.id() != start.id() && entity.entity_type() == shared_type
                    })
                    .cloned()
                    .collect();
                candidates.sort_by_key(Entity::id);
                fallback = Some((start, subgraph));
                candidates
            } else {
                self.evidence.add_entity(&start)?;
                self.note(format!(
                    "The {anchor} is a {}, which is neither a {} nor a {shared_type}.",
                    start.entity_type(),
                    member_type
                        .as_ref()
                        .map_or_else(|| "member".to_owned(), ToString::to_string),
                ));
                return Ok(Resolution::NotFound);
            }
        } else {
            let limit = self.limits.max_candidates;
            let search = EntitySearch::new()
                .with_entity_type(shared_type.clone())
                .with_limit(limit)?;
            let Some(found) = self.search(&search).await? else {
                return Ok(Resolution::Resolved);
            };
            if found.len() >= limit {
                self.truncate(format!(
                    "Bounded discovery: only the first {limit} {shared_type} entities (ordered by id) were inspected; more may exist."
                ));
            }
            found
        };

        let inward = reverse(direction);
        let mut groups = 0;
        for candidate in &candidates {
            let Some(subgraph) = self
                .neighbors(candidate.id(), inward, relationship_type)
                .await?
            else {
                break;
            };
            self.stats.candidates_inspected += 1;
            let (found, relationships) = subgraph.into_parts();
            let found: HashMap<EntityId, Entity> = found.into_iter().map(|e| (e.id(), e)).collect();
            // Member id → its relationships to the candidate, smallest id first.
            let mut members: BTreeMap<EntityId, Vec<Relationship>> = BTreeMap::new();
            for relationship in relationships {
                let other = other_end(&relationship, candidate.id());
                let Some(entity) = found.get(&other) else {
                    continue;
                };
                if other == candidate.id()
                    || member_type
                        .as_ref()
                        .is_some_and(|m| entity.entity_type() != m)
                {
                    continue;
                }
                members.entry(other).or_default().push(relationship);
            }
            if members.len() < 2 {
                continue;
            }
            // As many members as fit, each with all its relationships.
            let (entity_room, relationship_room) = self.evidence.remaining();
            let mut entity_need = usize::from(!self.evidence.contains_entity(candidate.id()));
            let mut relationship_need = 0;
            let mut fitting = Vec::new();
            for (id, links) in &members {
                let entity_cost = usize::from(!self.evidence.contains_entity(*id));
                if entity_need + entity_cost > entity_room
                    || relationship_need + links.len() > relationship_room
                {
                    break;
                }
                entity_need += entity_cost;
                relationship_need += links.len();
                fitting.push(*id);
            }
            if fitting.len() < 2 {
                self.budget_full();
                break;
            }
            if fitting.len() < members.len() {
                self.budget_full();
            }
            let mut entities: Vec<&Entity> = vec![candidate];
            let mut links: Vec<&Relationship> = Vec::new();
            for id in &fitting {
                entities.push(&found[id]);
                links.extend(members[id].iter());
            }
            if !self.evidence.add_all(&entities, &links)? {
                self.budget_full();
                break;
            }
            let first = fitting[0];
            for member in &fitting[1..] {
                self.evidence.add_path(EvidencePath::new(
                    vec![first, candidate.id(), *member],
                    vec![members[&first][0].id(), members[member][0].id()],
                ))?;
            }
            self.evidence.add_group(SharedGroup {
                shared: candidate.id(),
                members: fitting,
            })?;
            groups += 1;
        }

        if groups == 0 {
            let members = member_type
                .as_ref()
                .map_or_else(|| "entities".to_owned(), |m| format!("{m} entities"));
            self.note(format!(
                "None of the {} inspected {shared_type} entities connects two or more {members}.",
                self.stats.candidates_inspected
            ));
            // Scoped: show what was checked (the start and its candidates).
            if let Some((start, subgraph)) = fallback {
                let wanted: HashSet<EntityId> = candidates.iter().map(Entity::id).collect();
                let by_id: HashMap<EntityId, &Entity> =
                    subgraph.entities().iter().map(|e| (e.id(), e)).collect();
                self.evidence.add_entity(&start)?;
                for relationship in subgraph.relationships() {
                    let other = other_end(relationship, start.id());
                    if !wanted.contains(&other) {
                        continue;
                    }
                    if !self
                        .evidence
                        .add_all(&[&start, by_id[&other]], &[relationship])?
                    {
                        self.budget_full();
                        break;
                    }
                }
            }
        }
        Ok(Resolution::Resolved)
    }
}

/// Runs a validated plan against the GraphStore, within `limits`.
///
/// # Errors
///
/// A GraphStore failure (for example the backend is unavailable), or
/// retrieved data that breaks an evidence invariant (a GraphStore contract
/// violation).
pub async fn retrieve(
    store: &dyn GraphStore,
    graph_id: GraphId,
    plan: &RetrievalPlan,
    limits: &GraphRagLimits,
) -> Result<RetrievalOutcome, RetrievalError> {
    let limits = limits.sanitized();
    let mut retriever = Retriever {
        store,
        graph_id,
        limits,
        stats: RetrievalStats::default(),
        out_of_operations: false,
        limitations: Vec::new(),
        evidence: EvidenceBuilder::new(graph_id, &limits),
    };
    let resolution = match plan {
        RetrievalPlan::EntityLookup { target } => retriever.entity_lookup(target).await?,
        RetrievalPlan::Neighbors {
            center,
            direction,
            relationship_type,
            entity_type,
            depth,
        } => {
            retriever
                .neighbors_of(
                    center,
                    *direction,
                    relationship_type.as_ref(),
                    entity_type.as_ref(),
                    *depth,
                )
                .await?
        }
        RetrievalPlan::FindPath {
            source,
            target,
            direction,
            max_depth,
        } => {
            retriever
                .find_path(source, target, *direction, *max_depth)
                .await?
        }
        RetrievalPlan::SharedNeighbor {
            start,
            member_type,
            shared_type,
            relationship_type,
            direction,
        } => {
            retriever
                .shared_neighbor(
                    start.as_ref(),
                    member_type.as_ref(),
                    shared_type,
                    relationship_type.as_ref(),
                    *direction,
                )
                .await?
        }
        RetrievalPlan::Unsupported { .. } => Resolution::Resolved,
    };
    if retriever.evidence.is_full() {
        retriever.stats.truncated = true;
    }
    let evidence = retriever.evidence.finish();
    evidence.validate(graph_id)?;
    Ok(RetrievalOutcome {
        evidence,
        stats: retriever.stats,
        resolution,
        limitations: retriever.limitations,
    })
}
