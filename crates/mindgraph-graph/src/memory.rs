//! In-memory reference implementation of [`GraphStore`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use mindgraph_domain::{Entity, EntityId, GraphId, Relationship, RelationshipId};

use crate::validate;
use crate::{
    Direction, EntitySearch, GraphPath, GraphStore, GraphStoreError, NeighborQuery, PathQuery,
    Subgraph,
};

/// An in-memory [`GraphStore`]: the reference behaviour for the contract.
///
/// Intended for unit tests, local development and benchmarks. Data lives in
/// ordered maps behind a single `RwLock`, so results are deterministic.
/// Every write validates its whole input before changing anything, so a
/// failed write leaves the store unchanged. The lock is never held across
/// an `.await`.
#[derive(Debug, Default)]
pub struct MemoryGraphStore {
    graphs: RwLock<BTreeMap<GraphId, GraphData>>,
}

/// The contents of one graph.
#[derive(Debug, Default)]
struct GraphData {
    entities: BTreeMap<EntityId, Entity>,
    relationships: BTreeMap<RelationshipId, Relationship>,
    /// Relationship ids by source entity.
    outgoing: BTreeMap<EntityId, BTreeSet<RelationshipId>>,
    /// Relationship ids by target entity.
    incoming: BTreeMap<EntityId, BTreeSet<RelationshipId>>,
}

impl GraphData {
    /// Ids of the relationships touching `entity` in `direction`, in id order.
    fn incident(&self, entity: EntityId, direction: Direction) -> BTreeSet<RelationshipId> {
        let outgoing = self.outgoing.get(&entity).into_iter().flatten();
        let incoming = self.incoming.get(&entity).into_iter().flatten();
        match direction {
            Direction::Outgoing => outgoing.copied().collect(),
            Direction::Incoming => incoming.copied().collect(),
            Direction::Both => outgoing.chain(incoming).copied().collect(),
        }
    }

    fn insert_relationship(&mut self, relationship: Relationship) {
        let id = relationship.id();
        self.outgoing
            .entry(relationship.source_id())
            .or_default()
            .insert(id);
        self.incoming
            .entry(relationship.target_id())
            .or_default()
            .insert(id);
        self.relationships.insert(id, relationship);
    }

    /// Removes an entity and its incident relationships.
    fn remove_entity(&mut self, entity: EntityId) -> bool {
        if self.entities.remove(&entity).is_none() {
            return false;
        }
        for relationship_id in self.incident(entity, Direction::Both) {
            if let Some(relationship) = self.relationships.remove(&relationship_id) {
                for (index, endpoint) in [
                    (&mut self.outgoing, relationship.source_id()),
                    (&mut self.incoming, relationship.target_id()),
                ] {
                    if let Some(ids) = index.get_mut(&endpoint) {
                        ids.remove(&relationship_id);
                        if ids.is_empty() {
                            index.remove(&endpoint);
                        }
                    }
                }
            }
        }
        true
    }
}

impl MemoryGraphStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // A poisoned lock means a panic happened while it was held. Writes only
    // mutate after full validation, so the data is still consistent and it
    // is safe to keep using it.
    fn read(&self) -> RwLockReadGuard<'_, BTreeMap<GraphId, GraphData>> {
        self.graphs.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, BTreeMap<GraphId, GraphData>> {
        self.graphs.write().unwrap_or_else(PoisonError::into_inner)
    }
}

fn not_found(graph_id: GraphId, entity_id: EntityId) -> GraphStoreError {
    GraphStoreError::EntityNotFound {
        graph_id,
        entity_id,
    }
}

#[async_trait]
impl GraphStore for MemoryGraphStore {
    async fn create_entity(&self, entity: Entity) -> Result<(), GraphStoreError> {
        self.create_entities(vec![entity]).await
    }

    async fn create_entities(&self, entities: Vec<Entity>) -> Result<(), GraphStoreError> {
        let Some(graph_id) = validate::batch_graph(&entities, Entity::graph_id, "entities")? else {
            return Ok(());
        };
        validate::unique_ids(&entities, Entity::id, "entity")?;

        let mut graphs = self.write();
        let graph = graphs.entry(graph_id).or_default();
        if let Some(existing) = entities
            .iter()
            .find(|e| graph.entities.contains_key(&e.id()))
        {
            return Err(GraphStoreError::Conflict(format!(
                "entity {} already exists in graph {graph_id}",
                existing.id()
            )));
        }
        for entity in entities {
            graph.entities.insert(entity.id(), entity);
        }
        Ok(())
    }

    async fn create_relationship(&self, relationship: Relationship) -> Result<(), GraphStoreError> {
        self.create_relationships(vec![relationship]).await
    }

    async fn create_relationships(
        &self,
        relationships: Vec<Relationship>,
    ) -> Result<(), GraphStoreError> {
        let Some(graph_id) =
            validate::batch_graph(&relationships, Relationship::graph_id, "relationships")?
        else {
            return Ok(());
        };
        validate::unique_ids(&relationships, Relationship::id, "relationship")?;

        let mut graphs = self.write();
        let graph = graphs.entry(graph_id).or_default();
        for relationship in &relationships {
            for endpoint in [relationship.source_id(), relationship.target_id()] {
                if !graph.entities.contains_key(&endpoint) {
                    return Err(not_found(graph_id, endpoint));
                }
            }
        }
        if let Some(existing) = relationships
            .iter()
            .find(|r| graph.relationships.contains_key(&r.id()))
        {
            return Err(GraphStoreError::Conflict(format!(
                "relationship {} already exists in graph {graph_id}",
                existing.id()
            )));
        }
        for relationship in relationships {
            graph.insert_relationship(relationship);
        }
        Ok(())
    }

    async fn get_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<Option<Entity>, GraphStoreError> {
        Ok(self
            .read()
            .get(&graph_id)
            .and_then(|graph| graph.entities.get(&entity_id))
            .cloned())
    }

    async fn search_entities(
        &self,
        graph_id: GraphId,
        search: &EntitySearch,
    ) -> Result<Vec<Entity>, GraphStoreError> {
        let graphs = self.read();
        let Some(graph) = graphs.get(&graph_id) else {
            return Ok(Vec::new());
        };
        Ok(graph
            .entities
            .values()
            .filter(|entity| search.matches(entity))
            .take(search.limit())
            .cloned()
            .collect())
    }

    async fn get_neighbors(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
        query: &NeighborQuery,
    ) -> Result<Subgraph, GraphStoreError> {
        let graphs = self.read();
        let graph = graphs
            .get(&graph_id)
            .filter(|graph| graph.entities.contains_key(&entity_id))
            .ok_or_else(|| not_found(graph_id, entity_id))?;

        let relationships: Vec<Relationship> = graph
            .incident(entity_id, query.direction())
            .iter()
            .filter_map(|id| graph.relationships.get(id))
            .filter(|relationship| query.accepts(relationship))
            .take(query.limit())
            .cloned()
            .collect();
        let entity_ids: BTreeSet<EntityId> = relationships
            .iter()
            .flat_map(|r| [r.source_id(), r.target_id()])
            .chain([entity_id])
            .collect();
        let entities = entity_ids
            .iter()
            .filter_map(|id| graph.entities.get(id))
            .cloned()
            .collect();
        Ok(Subgraph::new(entities, relationships))
    }

    async fn find_path(
        &self,
        graph_id: GraphId,
        source: EntityId,
        target: EntityId,
        query: &PathQuery,
    ) -> Result<Option<GraphPath>, GraphStoreError> {
        let graphs = self.read();
        let graph = graphs
            .get(&graph_id)
            .ok_or_else(|| not_found(graph_id, source))?;
        for endpoint in [source, target] {
            if !graph.entities.contains_key(&endpoint) {
                return Err(not_found(graph_id, endpoint));
            }
        }

        // Iterative breadth-first search, one level per hop. Relationships
        // are visited in id order and the first discovery of an entity wins,
        // so the result is deterministic and each hop uses the smallest
        // relationship id between its two entities.
        let mut parents: BTreeMap<EntityId, (EntityId, RelationshipId)> = BTreeMap::new();
        let mut visited = BTreeSet::from([source]);
        let mut frontier = vec![source];
        let mut found = source == target;
        for _ in 0..query.max_depth() {
            if found || frontier.is_empty() {
                break;
            }
            let mut next = Vec::new();
            'level: for &current in &frontier {
                for relationship_id in graph.incident(current, query.direction()) {
                    let Some(relationship) = graph.relationships.get(&relationship_id) else {
                        continue;
                    };
                    let Some(reached) = query.direction().follow(relationship, current) else {
                        continue;
                    };
                    if visited.insert(reached) {
                        parents.insert(reached, (current, relationship_id));
                        if reached == target {
                            found = true;
                            break 'level;
                        }
                        next.push(reached);
                    }
                }
            }
            frontier = next;
        }
        if !found {
            return Ok(None);
        }

        let mut entity_ids = vec![target];
        let mut relationship_ids = Vec::new();
        let mut cursor = target;
        while let Some(&(parent, relationship_id)) = parents.get(&cursor) {
            relationship_ids.push(relationship_id);
            entity_ids.push(parent);
            cursor = parent;
        }
        entity_ids.reverse();
        relationship_ids.reverse();

        let entities = entity_ids
            .iter()
            .filter_map(|id| graph.entities.get(id))
            .cloned()
            .collect();
        let relationships = relationship_ids
            .iter()
            .filter_map(|id| graph.relationships.get(id))
            .cloned()
            .collect();
        GraphPath::new(entities, relationships).map(Some)
    }

    async fn get_subgraph(
        &self,
        graph_id: GraphId,
        entity_ids: &[EntityId],
    ) -> Result<Subgraph, GraphStoreError> {
        let requested = validate::subgraph_request(entity_ids)?;
        let graphs = self.read();
        let Some(graph) = graphs.get(&graph_id) else {
            return Ok(Subgraph::default());
        };
        let found: BTreeSet<EntityId> = requested
            .into_iter()
            .filter(|id| graph.entities.contains_key(id))
            .collect();
        let relationships = found
            .iter()
            .flat_map(|id| graph.incident(*id, Direction::Outgoing))
            .filter_map(|id| graph.relationships.get(&id))
            .filter(|relationship| found.contains(&relationship.target_id()))
            .cloned()
            .collect();
        let entities = found
            .iter()
            .filter_map(|id| graph.entities.get(id))
            .cloned()
            .collect();
        Ok(Subgraph::new(entities, relationships))
    }

    async fn delete_entity(
        &self,
        graph_id: GraphId,
        entity_id: EntityId,
    ) -> Result<bool, GraphStoreError> {
        Ok(self
            .write()
            .get_mut(&graph_id)
            .is_some_and(|graph| graph.remove_entity(entity_id)))
    }

    async fn clear_graph(&self, graph_id: GraphId) -> Result<(), GraphStoreError> {
        self.write().remove(&graph_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mindgraph_domain::{Properties, TypeName};

    use super::*;

    #[test]
    fn store_is_usable_as_shared_trait_object() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MemoryGraphStore>();
        let _store: Arc<dyn GraphStore> = Arc::new(MemoryGraphStore::new());
    }

    #[tokio::test]
    async fn concurrent_writers_do_not_lose_entities() {
        let store = Arc::new(MemoryGraphStore::new());
        let graph_id = GraphId::new();
        let tasks: Vec<_> = (0..32)
            .map(|_| {
                let store = Arc::clone(&store);
                tokio::spawn(async move {
                    let entity = Entity::new(
                        EntityId::new(),
                        graph_id,
                        TypeName::new("Node").unwrap(),
                        Properties::new(),
                    );
                    store.create_entity(entity).await
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        let all = store
            .search_entities(graph_id, &EntitySearch::new().with_limit(1_000).unwrap())
            .await
            .unwrap();
        assert_eq!(all.len(), 32);
    }

    #[tokio::test]
    async fn deleting_an_entity_cleans_adjacency_of_its_neighbours() {
        let store = MemoryGraphStore::new();
        let graph_id = GraphId::new();
        let node = |_: u8| {
            Entity::new(
                EntityId::new(),
                graph_id,
                TypeName::new("Node").unwrap(),
                Properties::new(),
            )
        };
        let (a, b) = (node(0), node(1));
        let ab = Relationship::new(
            RelationshipId::new(),
            graph_id,
            TypeName::new("LINKS").unwrap(),
            a.id(),
            b.id(),
            Properties::new(),
        );
        store
            .create_entities(vec![a.clone(), b.clone()])
            .await
            .unwrap();
        store.create_relationship(ab).await.unwrap();
        assert!(store.delete_entity(graph_id, b.id()).await.unwrap());

        let graphs = store.read();
        let graph = graphs.get(&graph_id).unwrap();
        assert!(graph.relationships.is_empty());
        assert!(graph.outgoing.is_empty(), "no dangling adjacency entries");
        assert!(graph.incoming.is_empty());
    }
}
