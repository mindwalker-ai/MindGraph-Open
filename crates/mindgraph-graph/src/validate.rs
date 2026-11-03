//! Request validation shared by every `GraphStore` implementation, so all
//! backends reject the same inputs with the same errors.

use std::collections::BTreeSet;
use std::fmt::Display;

use mindgraph_domain::{EntityId, GraphId};

use crate::{GraphStoreError, MAX_SUBGRAPH_ENTITIES};

/// Returns the single graph a batch belongs to, or `None` for an empty batch.
pub(crate) fn batch_graph<T>(
    items: &[T],
    graph_of: impl Fn(&T) -> GraphId,
    what: &str,
) -> Result<Option<GraphId>, GraphStoreError> {
    let Some(first) = items.first().map(&graph_of) else {
        return Ok(None);
    };
    if let Some(other) = items.iter().map(&graph_of).find(|graph| *graph != first) {
        return Err(GraphStoreError::InvalidInput(format!(
            "all {what} in a batch must belong to the same graph, found {first} and {other}"
        )));
    }
    Ok(Some(first))
}

/// Fails with a conflict if an id appears twice in a batch.
pub(crate) fn unique_ids<T, I>(
    items: &[T],
    id_of: impl Fn(&T) -> I,
    what: &str,
) -> Result<(), GraphStoreError>
where
    I: Ord + Display,
{
    let mut seen = BTreeSet::new();
    for id in items.iter().map(id_of) {
        if seen.contains(&id) {
            return Err(GraphStoreError::Conflict(format!(
                "{what} {id} appears more than once in the batch"
            )));
        }
        seen.insert(id);
    }
    Ok(())
}

/// Deduplicates a subgraph request and enforces its size limit.
pub(crate) fn subgraph_request(ids: &[EntityId]) -> Result<BTreeSet<EntityId>, GraphStoreError> {
    let requested: BTreeSet<EntityId> = ids.iter().copied().collect();
    if requested.len() > MAX_SUBGRAPH_ENTITIES {
        return Err(GraphStoreError::InvalidInput(format!(
            "a subgraph request may name at most {MAX_SUBGRAPH_ENTITIES} entities, got {}",
            requested.len()
        )));
    }
    Ok(requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_graph_detects_mixed_graphs() {
        let (a, b) = (GraphId::new(), GraphId::new());
        assert_eq!(batch_graph(&[a, a], |g| *g, "entities").unwrap(), Some(a));
        assert_eq!(
            batch_graph::<GraphId>(&[], |g| *g, "entities").unwrap(),
            None
        );
        assert!(matches!(
            batch_graph(&[a, b], |g| *g, "entities"),
            Err(GraphStoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn unique_ids_detects_repeats() {
        let id = EntityId::new();
        assert!(unique_ids(&[id, EntityId::new()], |i| *i, "entity").is_ok());
        assert!(matches!(
            unique_ids(&[id, id], |i| *i, "entity"),
            Err(GraphStoreError::Conflict(_))
        ));
    }

    #[test]
    fn subgraph_request_deduplicates_and_is_bounded() {
        let id = EntityId::new();
        assert_eq!(subgraph_request(&[id, id]).unwrap().len(), 1);
        let too_many: Vec<EntityId> = (0..=MAX_SUBGRAPH_ENTITIES)
            .map(|_| EntityId::new())
            .collect();
        assert!(matches!(
            subgraph_request(&too_many),
            Err(GraphStoreError::InvalidInput(_))
        ));
    }
}
