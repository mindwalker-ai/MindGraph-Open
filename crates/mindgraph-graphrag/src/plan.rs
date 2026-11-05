//! Retrieval intents (untrusted model output) and validated retrieval plans.
//!
//! The AI service interprets a question into a [`RetrievalIntent`]: a small
//! structured description using one of four bounded operations. It is never
//! a query language and never executable. [`crate::validate_intent`] turns
//! it into a [`RetrievalPlan`] whose names, ids, values and depths have been
//! checked against the graph's ontology and the limits; only a plan ever
//! reaches the GraphStore.

use std::collections::BTreeMap;
use std::fmt;

use mindgraph_domain::{EntityId, PropertyKey, PropertyValue, TypeName};
use mindgraph_graph::Direction;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The supported graph operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Entities by id, or by type and exact property values.
    EntityLookup,
    /// The neighbourhood of one entity, up to a few hops.
    Neighbors,
    /// A shortest path between two entities.
    FindPath,
    /// Entities connected through a shared entity (for example customers
    /// using the same device).
    SharedNeighbor,
    /// The question cannot be answered with the supported operations.
    Unsupported,
}

impl Operation {
    /// The wire name, for example `shared_neighbor`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EntityLookup => "entity_lookup",
            Self::Neighbors => "neighbors",
            Self::FindPath => "find_path",
            Self::SharedNeighbor => "shared_neighbor",
            Self::Unsupported => "unsupported",
        }
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How an intent names an entity. Exactly one form is valid: a known
/// entity (`ref`: `S1` for a selected entity, `H1` for an entity from an
/// earlier question of the conversation), an entity id named in the question, or
/// an entity type with exact property values (`match`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorIntent {
    /// A known entity: `S1` is the first entity of the question's scope,
    /// `H1` the first entity from earlier questions.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// An entity id (UUIDv7) given in the question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    /// The entity type, with `match`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// Exact property values (property name to value).
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    pub properties: Option<BTreeMap<String, Value>>,
}

/// A structured interpretation of a question, as returned by the AI
/// service. Untrusted until validated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalIntent {
    /// The operation.
    pub operation: Operation,
    /// The looked-up entity, the neighbourhood centre, the path source or
    /// the shared-neighbour starting point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AnchorIntent>,
    /// The path target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<AnchorIntent>,
    /// Neighbours: only entities of this type. Shared neighbour: the type of
    /// the entities that share something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// Shared neighbour: the type of the shared entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_entity_type: Option<String>,
    /// Only follow relationships of this type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship_type: Option<String>,
    /// `outgoing`, `incoming` or `both` (the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// Hops: neighbourhood depth or path depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u64>,
    /// A short user-visible explanation (or why the question is
    /// unsupported). Untrusted text; never interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A reference to an entity the question already knows (1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownRef {
    /// `S1`, …: an entity of the question's scope (selected in the explorer).
    Selected(usize),
    /// `H1`, …: an entity cited by an earlier question of the conversation.
    Earlier(usize),
}

impl fmt::Display for KnownRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selected(index) => write!(f, "S{index}"),
            Self::Earlier(index) => write!(f, "H{index}"),
        }
    }
}

/// A validated reference to an entity.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    /// A known entity id: a selected entity (`S1`, …), an entity from an
    /// earlier question (`H1`, …) or an id named in the question. Whether it
    /// exists in the graph is checked by retrieval.
    Entity {
        /// The entity id.
        id: EntityId,
        /// How the question referred to it, when by reference.
        reference: Option<KnownRef>,
    },
    /// Entities of a type with exact, typed property values (possibly none,
    /// which matches every entity of the type).
    Match {
        /// The entity type.
        entity_type: TypeName,
        /// The exact property values.
        filters: BTreeMap<PropertyKey, PropertyValue>,
    },
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entity {
                reference: Some(reference @ KnownRef::Selected(_)),
                ..
            } => write!(f, "selected entity {reference}"),
            Self::Entity {
                reference: Some(reference @ KnownRef::Earlier(_)),
                ..
            } => write!(f, "entity {reference} from an earlier question"),
            Self::Entity {
                id,
                reference: None,
            } => write!(f, "entity {id}"),
            Self::Match {
                entity_type,
                filters,
            } if filters.is_empty() => write!(f, "{entity_type} entities"),
            Self::Match {
                entity_type,
                filters,
            } => {
                write!(f, "{entity_type} where ")?;
                for (index, (key, value)) in filters.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" and ")?;
                    }
                    write!(f, "{key} = {}", value.to_json())?;
                }
                Ok(())
            }
        }
    }
}

/// A validated retrieval plan: only these shapes reach the GraphStore.
#[derive(Debug, Clone, PartialEq)]
pub enum RetrievalPlan {
    /// Entities by id, or by type and exact property values.
    EntityLookup {
        /// What to look up.
        target: Anchor,
    },
    /// The neighbourhood of one entity.
    Neighbors {
        /// The centre; must resolve to exactly one entity.
        center: Anchor,
        /// Which relationships to follow, seen from the centre.
        direction: Direction,
        /// Only follow relationships of this type.
        relationship_type: Option<TypeName>,
        /// Only report neighbours of this type (and the entities that
        /// connect them to the centre).
        entity_type: Option<TypeName>,
        /// Hops, `1..=max_traversal_depth`.
        depth: usize,
    },
    /// A shortest path.
    FindPath {
        /// The source; must resolve to exactly one entity.
        source: Anchor,
        /// The target; must resolve to exactly one entity.
        target: Anchor,
        /// Which relationships to follow.
        direction: Direction,
        /// Hops, `1..=max_path_depth`.
        max_depth: usize,
    },
    /// Entities connected through a shared entity.
    SharedNeighbor {
        /// A starting entity (one of the members or one shared entity), or
        /// `None` for bounded discovery over the shared type.
        start: Option<Anchor>,
        /// The type of the members; any type when `None` (with a start
        /// member, its own type).
        member_type: Option<TypeName>,
        /// The type of the shared entity.
        shared_type: TypeName,
        /// Only relationships of this type connect members and shared
        /// entities.
        relationship_type: Option<TypeName>,
        /// The direction from a member to the shared entity.
        direction: Direction,
    },
    /// The question cannot be answered with the supported operations.
    Unsupported {
        /// Why (model text, bounded).
        reason: String,
    },
}

impl RetrievalPlan {
    /// The operation of the plan.
    #[must_use]
    pub const fn operation(&self) -> Operation {
        match self {
            Self::EntityLookup { .. } => Operation::EntityLookup,
            Self::Neighbors { .. } => Operation::Neighbors,
            Self::FindPath { .. } => Operation::FindPath,
            Self::SharedNeighbor { .. } => Operation::SharedNeighbor,
            Self::Unsupported { .. } => Operation::Unsupported,
        }
    }
}

/// A validated plan, plus notes on what validation adjusted (for example an
/// ignored filter) and the model's short explanation.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedPlan {
    /// The plan.
    pub plan: RetrievalPlan,
    /// Adjustments made during validation, shown to the user and the model.
    pub notes: Vec<String>,
    /// The model's short explanation, bounded. Untrusted text.
    pub reason: Option<String>,
}

pub(crate) const fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Outgoing => "outgoing",
        Direction::Incoming => "incoming",
        Direction::Both => "both",
    }
}

/// A user-visible description of a validated plan (the "retrieval plan" of
/// an answer). It names schema types and the exact values that were
/// matched; it is never executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanSummary {
    /// The operation.
    pub operation: Operation,
    /// The looked-up entity, centre, path source or starting point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The path target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Neighbour or member type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// Shared entity type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_entity_type: Option<String>,
    /// Relationship type filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship_type: Option<String>,
    /// `outgoing`, `incoming` or `both`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<&'static str>,
    /// Hops.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,
    /// The model's short explanation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Adjustments made during validation.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl ValidatedPlan {
    /// The user-visible description.
    #[must_use]
    pub fn summary(&self) -> PlanSummary {
        let name = |name: &Option<TypeName>| name.as_ref().map(ToString::to_string);
        let mut summary = PlanSummary {
            operation: self.plan.operation(),
            source: None,
            target: None,
            entity_type: None,
            shared_entity_type: None,
            relationship_type: None,
            direction: None,
            max_depth: None,
            reason: self.reason.clone(),
            notes: self.notes.clone(),
        };
        match &self.plan {
            RetrievalPlan::EntityLookup { target } => summary.source = Some(target.to_string()),
            RetrievalPlan::Neighbors {
                center,
                direction,
                relationship_type,
                entity_type,
                depth,
            } => {
                summary.source = Some(center.to_string());
                summary.direction = Some(direction_name(*direction));
                summary.relationship_type = name(relationship_type);
                summary.entity_type = name(entity_type);
                summary.max_depth = Some(*depth);
            }
            RetrievalPlan::FindPath {
                source,
                target,
                direction,
                max_depth,
            } => {
                summary.source = Some(source.to_string());
                summary.target = Some(target.to_string());
                summary.direction = Some(direction_name(*direction));
                summary.max_depth = Some(*max_depth);
            }
            RetrievalPlan::SharedNeighbor {
                start,
                member_type,
                shared_type,
                relationship_type,
                direction,
            } => {
                summary.source = start.as_ref().map(ToString::to_string);
                summary.entity_type = name(member_type);
                summary.shared_entity_type = Some(shared_type.to_string());
                summary.relationship_type = name(relationship_type);
                summary.direction = Some(direction_name(*direction));
            }
            RetrievalPlan::Unsupported { .. } => {}
        }
        summary
    }
}
