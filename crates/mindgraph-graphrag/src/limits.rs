//! Resource limits of one GraphRAG question.
//!
//! Every retrieval runs inside these bounds. None of them raises a
//! GraphStore bound: per-call limits stay within `EntitySearch::MAX_LIMIT`,
//! `NeighborQuery::MAX_LIMIT` and `PathQuery::MAX_DEPTH`.

use mindgraph_graph::{EntitySearch, NeighborQuery, PathQuery};

/// Longest accepted question, in characters.
pub const MAX_QUESTION_CHARS: usize = 2_000;

/// Most entities a question may be scoped to.
pub const MAX_SEED_ENTITIES: usize = 5;

/// Most earlier questions a follow-up carries as conversation context.
pub const MAX_CONVERSATION_TURNS: usize = 3;

/// Most entity ids one earlier question may carry.
pub const MAX_TURN_ENTITIES: usize = 10;

/// Most entities from earlier questions (`H1`, …) a follow-up may refer to.
pub const MAX_EARLIER_ENTITIES: usize = 10;

/// Most candidate entities shown when an anchor is ambiguous.
pub const MAX_AMBIGUOUS_CANDIDATES: usize = 10;

/// Most exact-match properties in one anchor.
pub const MAX_MATCH_PROPERTIES: usize = 10;

/// The bounds of one GraphRAG retrieval and its model context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphRagLimits {
    /// Most candidates read by one search (entity lookups, shared-neighbour
    /// discovery). At most `EntitySearch::MAX_LIMIT`.
    pub max_candidates: usize,
    /// Most relationships read by one neighbourhood call. At most
    /// `NeighborQuery::MAX_LIMIT`.
    pub max_neighbors_per_call: usize,
    /// Deepest neighbourhood traversal, in hops.
    pub max_traversal_depth: usize,
    /// Deepest shortest-path search, in hops. At most `PathQuery::MAX_DEPTH`.
    pub max_path_depth: usize,
    /// Most entities in an evidence graph.
    pub max_evidence_entities: usize,
    /// Most relationships in an evidence graph.
    pub max_evidence_relationships: usize,
    /// Largest serialised model context, in bytes.
    pub max_context_bytes: usize,
    /// Most GraphStore calls for one question.
    pub max_operations: usize,
    /// Longest property text in the model context, in characters.
    pub max_property_chars: usize,
    /// Most properties of one entity or relationship in the model context.
    pub max_properties_per_item: usize,
}

impl Default for GraphRagLimits {
    fn default() -> Self {
        Self {
            max_candidates: EntitySearch::DEFAULT_LIMIT,
            max_neighbors_per_call: NeighborQuery::DEFAULT_LIMIT,
            max_traversal_depth: 3,
            max_path_depth: PathQuery::MAX_DEPTH,
            max_evidence_entities: 150,
            max_evidence_relationships: 300,
            max_context_bytes: 48 * 1024,
            max_operations: 100,
            max_property_chars: 200,
            max_properties_per_item: 24,
        }
    }
}

impl GraphRagLimits {
    /// The limits with every per-call bound clamped to the GraphStore's own
    /// bounds and every count at least 1, so a misconfiguration can never
    /// widen a GraphStore query.
    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            max_candidates: self.max_candidates.clamp(1, EntitySearch::MAX_LIMIT),
            max_neighbors_per_call: self
                .max_neighbors_per_call
                .clamp(1, NeighborQuery::MAX_LIMIT),
            max_traversal_depth: self.max_traversal_depth.max(1),
            max_path_depth: self.max_path_depth.clamp(1, PathQuery::MAX_DEPTH),
            max_evidence_entities: self.max_evidence_entities.max(2),
            max_evidence_relationships: self.max_evidence_relationships.max(1),
            max_context_bytes: self.max_context_bytes.max(4 * 1024),
            max_operations: self.max_operations.max(1),
            max_property_chars: self.max_property_chars.max(16),
            max_properties_per_item: self.max_properties_per_item.max(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_stay_within_graph_store_bounds() {
        let limits = GraphRagLimits::default();
        assert_eq!(limits, limits.sanitized());
        let wide = GraphRagLimits {
            max_candidates: 50_000,
            max_neighbors_per_call: 0,
            max_path_depth: 99,
            ..limits
        }
        .sanitized();
        assert_eq!(wide.max_candidates, EntitySearch::MAX_LIMIT);
        assert_eq!(wide.max_neighbors_per_call, 1);
        assert_eq!(wide.max_path_depth, PathQuery::MAX_DEPTH);
    }
}
