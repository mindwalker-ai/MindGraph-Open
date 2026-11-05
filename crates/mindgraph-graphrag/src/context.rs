//! Deterministic, bounded serialisation of an evidence graph for the model.
//!
//! The context is compact JSON with sorted keys: entities, relationships,
//! paths and shared groups, each with its reference (`E1`, `R1`, `P1`), plus
//! what the retrieval covered. Internal metadata (graph ids, MindGraph ids,
//! backend details) is never included; the model sees references only.
//!
//! When the context is too large, it is reduced in steps: shorter property
//! values and fewer properties first, then whole items, keeping the paths
//! and shared groups (and the relationships and endpoints they need) before
//! anything else. The reduced evidence graph is returned and becomes the
//! answer's evidence, so the model can never cite something the response
//! does not contain, and no relationship is dropped while a kept claim
//! depends on it.

use std::collections::HashSet;

use mindgraph_domain::{EntityId, Properties, PropertyValue, RelationshipId};
use serde_json::{Map, Value, json};

use crate::evidence::EvidenceGraph;
use crate::limits::GraphRagLimits;
use crate::plan::Operation;

/// What the retrieval covered, for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMeta<'a> {
    /// `selected_entities` or `bounded_graph`.
    pub scope: &'a str,
    /// The plan's operation.
    pub operation: Operation,
    /// Whether the requested scope was read completely.
    pub complete: bool,
    /// Candidates inspected.
    pub candidates_inspected: usize,
    /// User-visible limitations.
    pub limitations: &'a [String],
}

/// A serialised context and the evidence it contains.
#[derive(Debug, Clone, PartialEq)]
pub struct SerializedContext {
    /// The context object sent to the model.
    pub value: Value,
    /// Its compact JSON size in bytes.
    pub bytes: usize,
    /// The evidence actually in the context (reduced when needed).
    pub evidence: EvidenceGraph,
    /// Whether properties or items were left out to fit the budget.
    pub reduced: bool,
}

#[derive(Debug, Clone, Copy)]
struct Detail {
    max_chars: usize,
    max_properties: usize,
}

/// Identity- and name-like properties first, then alphabetical.
fn property_rank(key: &str) -> (u8, &str) {
    let rank = if key == "id" || key.ends_with("_id") {
        0
    } else if matches!(
        key,
        "name" | "display_name" | "title" | "full_name" | "label"
    ) {
        1
    } else {
        2
    };
    (rank, key)
}

fn text(value: &str, max_chars: usize) -> Value {
    if value.chars().count() <= max_chars {
        return Value::String(value.to_owned());
    }
    let mut kept: String = value.chars().take(max_chars).collect();
    kept.push('…');
    Value::String(kept)
}

fn render_value(value: &PropertyValue, detail: Detail) -> Value {
    match value {
        PropertyValue::String(value) => text(value, detail.max_chars),
        PropertyValue::StringArray(values) => Value::Array(
            values
                .iter()
                .take(10)
                .map(|value| text(value, detail.max_chars))
                .collect(),
        ),
        other => other.to_json(),
    }
}

fn render_properties(properties: &Properties, detail: Detail) -> Value {
    let mut keys: Vec<_> = properties.iter().collect();
    keys.sort_by(|(a, _), (b, _)| property_rank(a.as_str()).cmp(&property_rank(b.as_str())));
    let map: Map<String, Value> = keys
        .into_iter()
        .take(detail.max_properties)
        .map(|(key, value)| (key.to_string(), render_value(value, detail)))
        .collect();
    Value::Object(map)
}

/// The properties of a selected entity as the model sees them: identity-
/// and name-like keys first, at most `limits.max_properties_per_item`
/// properties, texts cut at `limits.max_property_chars` characters.
#[must_use]
pub fn model_properties(properties: &Properties, limits: &GraphRagLimits) -> Value {
    let limits = limits.sanitized();
    render_properties(
        properties,
        Detail {
            max_chars: limits.max_property_chars,
            max_properties: limits.max_properties_per_item,
        },
    )
}

fn render(
    evidence: &EvidenceGraph,
    meta: &ContextMeta<'_>,
    detail: Detail,
    reduced: bool,
) -> Value {
    let entity_ref = |id: EntityId| evidence.entity_ref(id).map(|r| r.to_string());
    let relationship_ref =
        |id: RelationshipId| evidence.relationship_ref(id).map(|r| r.to_string());
    let entities: Vec<Value> = evidence
        .entities()
        .iter()
        .enumerate()
        .map(|(index, entity)| {
            json!({
                "ref": format!("E{}", index + 1),
                "type": entity.entity_type().as_str(),
                "properties": render_properties(entity.properties(), detail),
            })
        })
        .collect();
    let relationships: Vec<Value> = evidence
        .relationships()
        .iter()
        .enumerate()
        .map(|(index, relationship)| {
            json!({
                "ref": format!("R{}", index + 1),
                "type": relationship.relationship_type().as_str(),
                "source": entity_ref(relationship.source_id()),
                "target": entity_ref(relationship.target_id()),
                "properties": render_properties(relationship.properties(), detail),
            })
        })
        .collect();
    let paths: Vec<Value> = evidence
        .paths()
        .iter()
        .enumerate()
        .map(|(index, path)| {
            json!({
                "ref": format!("P{}", index + 1),
                "entities": path.entity_ids().iter().map(|id| entity_ref(*id)).collect::<Vec<_>>(),
                "relationships": path.relationship_ids().iter().map(|id| relationship_ref(*id)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let groups: Vec<Value> = evidence
        .groups()
        .iter()
        .map(|group| {
            json!({
                "shared": entity_ref(group.shared),
                "members": group.members.iter().map(|id| entity_ref(*id)).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "entities": entities,
        "relationships": relationships,
        "paths": paths,
        "shared_groups": groups,
        "retrieval": {
            "scope": meta.scope,
            "operation": meta.operation.as_str(),
            "complete": meta.complete,
            "candidates_inspected": meta.candidates_inspected,
            "context_reduced": reduced,
            "limitations": meta.limitations,
        },
    })
}

fn size(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

/// The items to keep when whole items must go: paths and groups first (with
/// everything they need), then other relationships with their endpoints,
/// then remaining entities, while the estimated size fits.
fn shrink(
    evidence: &EvidenceGraph,
    meta: &ContextMeta<'_>,
    detail: Detail,
    max_bytes: usize,
) -> EvidenceGraph {
    let base = size(&render(&EvidenceGraph::default(), meta, detail, true));
    let single = |entity_ids: &[EntityId], relationship_ids: &[RelationshipId]| {
        let piece = evidence.retain(
            &entity_ids.iter().copied().collect(),
            &relationship_ids.iter().copied().collect(),
        );
        size(&render(&piece, meta, detail, true)).saturating_sub(base)
    };
    let mut budget = max_bytes.saturating_sub(base);
    let mut entities: HashSet<EntityId> = HashSet::new();
    let mut relationships: HashSet<RelationshipId> = HashSet::new();
    let mut take = |entity_ids: Vec<EntityId>,
                    relationship_ids: Vec<RelationshipId>,
                    entities: &mut HashSet<EntityId>,
                    relationships: &mut HashSet<RelationshipId>| {
        let new_entities: Vec<EntityId> = entity_ids
            .into_iter()
            .filter(|id| !entities.contains(id))
            .collect();
        let new_relationships: Vec<RelationshipId> = relationship_ids
            .into_iter()
            .filter(|id| !relationships.contains(id))
            .collect();
        // A path reference costs about as much as its id list; 64 bytes is generous.
        let cost = single(&new_entities, &new_relationships) + 64;
        if cost > budget {
            return false;
        }
        budget -= cost;
        entities.extend(new_entities);
        relationships.extend(new_relationships);
        true
    };
    for path in evidence.paths() {
        take(
            path.entity_ids().to_vec(),
            path.relationship_ids().to_vec(),
            &mut entities,
            &mut relationships,
        );
    }
    for group in evidence.groups() {
        let mut ids = vec![group.shared];
        ids.extend(&group.members);
        take(ids, Vec::new(), &mut entities, &mut relationships);
    }
    for relationship in evidence.relationships() {
        take(
            vec![relationship.source_id(), relationship.target_id()],
            vec![relationship.id()],
            &mut entities,
            &mut relationships,
        );
    }
    for entity in evidence.entities() {
        take(
            vec![entity.id()],
            Vec::new(),
            &mut entities,
            &mut relationships,
        );
    }
    evidence.retain(&entities, &relationships)
}

/// Serialises `evidence` within `limits.max_context_bytes`, reducing it if
/// needed. The same input always yields the same bytes.
#[must_use]
pub fn build_context(
    evidence: &EvidenceGraph,
    meta: &ContextMeta<'_>,
    limits: &GraphRagLimits,
) -> SerializedContext {
    let limits = limits.sanitized();
    let full = Detail {
        max_chars: limits.max_property_chars,
        max_properties: limits.max_properties_per_item,
    };
    let value = render(evidence, meta, full, false);
    let bytes = size(&value);
    if bytes <= limits.max_context_bytes {
        return SerializedContext {
            value,
            bytes,
            evidence: evidence.clone(),
            reduced: false,
        };
    }
    let compact = Detail {
        max_chars: full.max_chars.min(80),
        max_properties: full.max_properties.min(4),
    };
    let value = render(evidence, meta, compact, true);
    let bytes = size(&value);
    if bytes <= limits.max_context_bytes {
        return SerializedContext {
            value,
            bytes,
            evidence: evidence.clone(),
            reduced: true,
        };
    }
    let mut kept = shrink(evidence, meta, compact, limits.max_context_bytes);
    let mut value = render(&kept, meta, compact, true);
    // The estimate is per item; drop from the end until the whole fits.
    while size(&value) > limits.max_context_bytes && !kept.is_empty() {
        let entities: HashSet<EntityId> = kept
            .entities()
            .iter()
            .take(kept.entities().len() - 1)
            .map(mindgraph_domain::Entity::id)
            .collect();
        let relationships: HashSet<RelationshipId> = kept
            .relationships()
            .iter()
            .map(mindgraph_domain::Relationship::id)
            .collect();
        kept = kept.retain(&entities, &relationships);
        value = render(&kept, meta, compact, true);
    }
    let bytes = size(&value);
    SerializedContext {
        value,
        bytes,
        evidence: kept,
        reduced: true,
    }
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::{Entity, GraphId, Relationship, TypeName};

    use super::*;
    use crate::evidence::{EvidenceBuilder, EvidencePath};

    fn meta() -> ContextMeta<'static> {
        ContextMeta {
            scope: "bounded_graph",
            operation: Operation::SharedNeighbor,
            complete: true,
            candidates_inspected: 1,
            limitations: &[],
        }
    }

    fn entity(graph_id: GraphId, entity_type: &str, properties: &[(&str, &str)]) -> Entity {
        let mut props = Properties::new();
        for (key, value) in properties {
            props
                .insert(mindgraph_domain::PropertyKey::new(*key).unwrap(), *value)
                .unwrap();
        }
        Entity::new(
            EntityId::new(),
            graph_id,
            TypeName::new(entity_type).unwrap(),
            props,
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

    #[test]
    fn the_context_is_deterministic_and_uses_references_only() {
        let graph_id = GraphId::new();
        let alice = entity(
            graph_id,
            "Customer",
            &[("name", "Alice"), ("customer_id", "C001")],
        );
        let device = entity(graph_id, "Device", &[("device_id", "DEVICE-921")]);
        let uses = link(&alice, &device);
        let mut builder = EvidenceBuilder::new(graph_id, &GraphRagLimits::default());
        builder.add_all(&[&alice, &device], &[&uses]).unwrap();
        builder
            .add_path(EvidencePath::new(
                vec![alice.id(), device.id()],
                vec![uses.id()],
            ))
            .unwrap();
        let evidence = builder.finish();
        let first = build_context(&evidence, &meta(), &GraphRagLimits::default());
        let second = build_context(&evidence, &meta(), &GraphRagLimits::default());
        let text = serde_json::to_string(&first.value).unwrap();
        assert_eq!(text, serde_json::to_string(&second.value).unwrap());
        assert!(!first.reduced);
        assert!(!text.contains(&alice.id().to_string()), "no MindGraph ids");
        assert!(!text.contains(&graph_id.to_string()), "no graph id");
        assert_eq!(first.value["relationships"][0]["source"], "E1");
        assert_eq!(first.value["paths"][0]["entities"], json!(["E1", "E2"]));
        assert_eq!(first.bytes, text.len());
    }

    #[test]
    fn large_evidence_is_reduced_without_breaking_claims() {
        let graph_id = GraphId::new();
        let device = entity(graph_id, "Device", &[("device_id", "DEVICE-921")]);
        let long = "x".repeat(500);
        let customers: Vec<Entity> = (0..120)
            .map(|i| {
                entity(
                    graph_id,
                    "Customer",
                    &[
                        ("customer_id", &format!("C{i:03}")),
                        ("notes", &long),
                        ("bio", &long),
                    ],
                )
            })
            .collect();
        let links: Vec<Relationship> = customers.iter().map(|c| link(c, &device)).collect();
        let mut builder = EvidenceBuilder::new(graph_id, &GraphRagLimits::default());
        builder.add_entity(&device).unwrap();
        for (customer, uses) in customers.iter().zip(&links) {
            builder.add_all(&[customer], &[uses]).unwrap();
        }
        builder
            .add_path(EvidencePath::new(
                vec![customers[0].id(), device.id(), customers[1].id()],
                vec![links[0].id(), links[1].id()],
            ))
            .unwrap();
        let evidence = builder.finish();
        let limits = GraphRagLimits {
            max_context_bytes: 8 * 1024,
            ..GraphRagLimits::default()
        };
        let context = build_context(&evidence, &meta(), &limits);
        assert!(context.reduced);
        assert!(
            context.bytes <= limits.max_context_bytes,
            "{}",
            context.bytes
        );
        assert_eq!(context.evidence.paths().len(), 1, "the path claim survives");
        context.evidence.validate(graph_id).unwrap();
        assert!(context.evidence.entities().len() < evidence.entities().len());
        assert_eq!(
            context.value["entities"].as_array().unwrap().len(),
            context.evidence.entities().len(),
            "the context and the evidence agree"
        );
    }
}
