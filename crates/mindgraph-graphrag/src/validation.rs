//! Validation of untrusted retrieval intents against the ontology and the
//! limits.
//!
//! The model's output is treated like any other untrusted input: names must
//! be ontology types, property names must belong to the named type, values
//! are typed by the ontology, ids must be MindGraph ids, references to
//! selected entities (`S1`) and to entities of earlier questions (`H1`)
//! must exist and depths must stay within the limits. Nothing
//! the model writes is ever executed or interpolated into a query.

use std::collections::BTreeMap;

use mindgraph_domain::{
    EntityId, Ontology, PropertyKey, PropertyType, PropertyValue, RelationshipType, TypeName,
};
use mindgraph_graph::{Direction, PathQuery};
use serde_json::Value;

use crate::PlanError;
use crate::limits::{GraphRagLimits, MAX_MATCH_PROPERTIES};
use crate::plan::{
    Anchor, AnchorIntent, KnownRef, Operation, RetrievalIntent, RetrievalPlan, ValidatedPlan,
};

/// Longest model explanation kept, in characters.
const MAX_REASON_CHARS: usize = 300;

/// What an intent is validated against.
#[derive(Debug, Clone, Copy)]
pub struct PlanContext<'a> {
    /// The graph's active ontology.
    pub ontology: &'a Ontology,
    /// The question's selected entities: `S1` is `selected[0]`.
    pub selected: &'a [EntityId],
    /// Entities cited by earlier questions of the conversation (follow-ups):
    /// `H1` is `earlier[0]`. They exist in this graph (checked by the caller).
    pub earlier: &'a [EntityId],
    /// The retrieval limits.
    pub limits: &'a GraphRagLimits,
}

fn invalid(message: impl Into<String>) -> PlanError {
    PlanError::Invalid(message.into())
}

/// A model-provided name, echoed only if it is a well-formed machine name.
fn shown(raw: &str) -> String {
    TypeName::new(raw.trim())
        .map_or_else(|_| "an invalid name".to_owned(), |name| format!("`{name}`"))
}

fn bounded(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    if text.is_empty() {
        return None;
    }
    let mut kept: String = text.chars().take(MAX_REASON_CHARS).collect();
    if kept.len() < text.len() {
        kept.push('…');
    }
    Some(kept)
}

fn entity_type(ontology: &Ontology, raw: Option<&str>) -> Result<Option<TypeName>, PlanError> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let name = TypeName::new(raw).map_err(|_| invalid("an entity type name is invalid"))?;
    if ontology.entity_type(name.as_str()).is_none() {
        return Err(invalid(format!(
            "entity type {} is not in the graph's ontology",
            shown(raw)
        )));
    }
    Ok(Some(name))
}

fn relationship_type<'o>(
    ontology: &'o Ontology,
    raw: Option<&str>,
) -> Result<Option<&'o RelationshipType>, PlanError> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    ontology.relationship_type(raw).map(Some).ok_or_else(|| {
        invalid(format!(
            "relationship type {} is not in the graph's ontology",
            shown(raw)
        ))
    })
}

fn direction(raw: Option<&str>) -> Result<Direction, PlanError> {
    match raw.map(str::trim) {
        None | Some("" | "both") => Ok(Direction::Both),
        Some("outgoing") => Ok(Direction::Outgoing),
        Some("incoming") => Ok(Direction::Incoming),
        Some(_) => Err(invalid("direction must be outgoing, incoming or both")),
    }
}

fn depth(raw: Option<u64>, default: usize, max: usize, what: &str) -> Result<usize, PlanError> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    let value = usize::try_from(raw).unwrap_or(usize::MAX);
    if value == 0 {
        return Err(invalid(format!("{what} must be at least 1")));
    }
    if value > max {
        return Err(PlanError::LimitExceeded(format!(
            "{what} may be at most {max}, the plan asked for {raw}"
        )));
    }
    Ok(value)
}

/// A filter value typed by the ontology. Integers also accept decimal
/// strings and strings accept numbers, because models write both.
fn typed_value(
    key: &PropertyKey,
    value: &Value,
    expected: PropertyType,
) -> Result<PropertyValue, PlanError> {
    let mismatch = || {
        invalid(format!(
            "the value of `{key}` does not match its type ({expected})"
        ))
    };
    match (expected, value) {
        (_, Value::Null) => Err(invalid(format!("the value of `{key}` must not be null"))),
        (PropertyType::Integer, Value::String(text)) => text
            .trim()
            .parse::<i64>()
            .map(PropertyValue::Integer)
            .map_err(|_| mismatch()),
        (PropertyType::String, Value::Number(number)) => {
            Ok(PropertyValue::String(number.to_string()))
        }
        (expected, value) => {
            PropertyValue::from_json(key.as_str(), value, expected).map_err(|_| mismatch())
        }
    }
}

/// `S1` or `H1` (1-based, no leading zeros).
fn parse_known_ref(raw: &str) -> Option<KnownRef> {
    let kind = raw.chars().next()?;
    let digits = &raw[kind.len_utf8()..];
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let index = digits.parse::<usize>().ok()?;
    match kind {
        'S' => Some(KnownRef::Selected(index)),
        'H' => Some(KnownRef::Earlier(index)),
        _ => None,
    }
}

/// The 1-based position of `id` in `ids`.
fn position(ids: &[EntityId], id: EntityId) -> Option<usize> {
    ids.iter()
        .position(|known| *known == id)
        .map(|index| index + 1)
}

fn anchor(raw: &AnchorIntent, context: &PlanContext<'_>, what: &str) -> Result<Anchor, PlanError> {
    let forms = [
        raw.reference.is_some(),
        raw.entity_id.is_some(),
        raw.entity_type.is_some(),
    ];
    if forms.iter().filter(|set| **set).count() != 1 {
        return Err(invalid(format!(
            "the {what} must name exactly one of: a known entity (ref), an entity_id, or an entity_type with match"
        )));
    }
    if let Some(reference) = raw.reference.as_deref().map(str::trim) {
        let parsed = parse_known_ref(reference)
            .ok_or_else(|| invalid(format!("the {what} has an invalid entity reference")))?;
        let (known, index, label) = match parsed {
            KnownRef::Selected(index) => (context.selected, index, "selected entities"),
            KnownRef::Earlier(index) => (context.earlier, index, "entities from earlier questions"),
        };
        let id = known.get(index - 1).ok_or_else(|| {
            invalid(format!(
                "the {what} refers to {parsed}, but there are {} {label}",
                known.len()
            ))
        })?;
        return Ok(Anchor::Entity {
            id: *id,
            reference: Some(parsed),
        });
    }
    if let Some(raw_id) = raw.entity_id.as_deref() {
        let id: EntityId = raw_id
            .trim()
            .parse()
            .map_err(|_| invalid(format!("the {what} has an invalid entity_id")))?;
        let reference = position(context.selected, id)
            .map(KnownRef::Selected)
            .or_else(|| position(context.earlier, id).map(KnownRef::Earlier));
        return Ok(Anchor::Entity { id, reference });
    }
    let entity_type = entity_type(context.ontology, raw.entity_type.as_deref())?
        .ok_or_else(|| invalid(format!("the {what} needs an entity type")))?;
    let schema = context
        .ontology
        .entity_type(entity_type.as_str())
        .map(mindgraph_domain::EntityType::properties_schema);
    let properties = raw
        .properties
        .as_ref()
        .map_or(&BTreeMap::new(), |p| p)
        .clone();
    if properties.len() > MAX_MATCH_PROPERTIES {
        return Err(PlanError::LimitExceeded(format!(
            "an anchor may match at most {MAX_MATCH_PROPERTIES} properties"
        )));
    }
    let mut filters = BTreeMap::new();
    for (raw_key, value) in &properties {
        let key = PropertyKey::new(raw_key.trim())
            .map_err(|_| invalid(format!("the {what} names an invalid property")))?;
        let expected = schema
            .and_then(|schema| schema.get(key.as_str()))
            .map(|definition| definition.property_type)
            .ok_or_else(|| {
                invalid(format!(
                    "property `{key}` is not defined for `{entity_type}`"
                ))
            })?;
        filters.insert(key.clone(), typed_value(&key, value, expected)?);
    }
    Ok(Anchor::Match {
        entity_type,
        filters,
    })
}

fn required<'i>(
    raw: Option<&'i AnchorIntent>,
    what: &str,
    operation: Operation,
) -> Result<&'i AnchorIntent, PlanError> {
    raw.ok_or_else(|| invalid(format!("{operation} needs a {what}")))
}

fn connects(relationship: &RelationshipType, a: &TypeName, b: &TypeName) -> bool {
    let (source, target) = (
        relationship.source_entity_type(),
        relationship.target_entity_type(),
    );
    (source == a && target == b) || (source == b && target == a)
}

fn touches(relationship: &RelationshipType, entity_type: &TypeName) -> bool {
    relationship.source_entity_type() == entity_type
        || relationship.target_entity_type() == entity_type
}

/// Validates an untrusted retrieval intent.
///
/// # Errors
///
/// [`PlanError::Invalid`] for unknown types or properties, malformed ids or
/// references, wrongly typed values, a missing anchor or an impossible
/// relationship type; [`PlanError::LimitExceeded`] for depths or match sizes
/// beyond the limits.
pub fn validate_intent(
    intent: &RetrievalIntent,
    context: &PlanContext<'_>,
) -> Result<ValidatedPlan, PlanError> {
    let limits = context.limits;
    let ontology = context.ontology;
    let operation = intent.operation;
    let reason = bounded(intent.reason.as_deref());
    let mut notes = Vec::new();
    let plan = match operation {
        Operation::Unsupported => RetrievalPlan::Unsupported {
            reason: reason.clone().unwrap_or_else(|| {
                "the question does not map to a supported graph operation".to_owned()
            }),
        },
        Operation::EntityLookup => {
            let target = anchor(
                required(intent.source.as_ref(), "source", operation)?,
                context,
                "source",
            )?;
            RetrievalPlan::EntityLookup { target }
        }
        Operation::Neighbors => {
            let center = anchor(
                required(intent.source.as_ref(), "source", operation)?,
                context,
                "source",
            )?;
            let relationship = relationship_type(ontology, intent.relationship_type.as_deref())?;
            RetrievalPlan::Neighbors {
                center,
                direction: direction(intent.direction.as_deref())?,
                relationship_type: relationship.map(|r| r.name().clone()),
                entity_type: entity_type(ontology, intent.entity_type.as_deref())?,
                depth: depth(
                    intent.max_depth,
                    1,
                    limits.max_traversal_depth,
                    "the neighbourhood depth",
                )?,
            }
        }
        Operation::FindPath => {
            let source = anchor(
                required(intent.source.as_ref(), "source", operation)?,
                context,
                "source",
            )?;
            let target = anchor(
                required(intent.target.as_ref(), "target", operation)?,
                context,
                "target",
            )?;
            if relationship_type(ontology, intent.relationship_type.as_deref())?.is_some() {
                notes.push(
                    "Shortest-path search cannot be restricted to one relationship type; the filter was ignored."
                        .to_owned(),
                );
            }
            RetrievalPlan::FindPath {
                source,
                target,
                direction: direction(intent.direction.as_deref())?,
                max_depth: depth(
                    intent.max_depth,
                    PathQuery::DEFAULT_MAX_DEPTH.min(limits.max_path_depth),
                    limits.max_path_depth,
                    "the path depth",
                )?,
            }
        }
        Operation::SharedNeighbor => {
            let shared_type = entity_type(ontology, intent.shared_entity_type.as_deref())?
                .ok_or_else(|| invalid("shared_neighbor needs a shared_entity_type"))?;
            let member_type = entity_type(ontology, intent.entity_type.as_deref())?;
            let relationship = relationship_type(ontology, intent.relationship_type.as_deref())?;
            if let Some(relationship) = relationship {
                let fits = member_type.as_ref().map_or_else(
                    || touches(relationship, &shared_type),
                    |member| connects(relationship, member, &shared_type),
                );
                if !fits {
                    return Err(invalid(format!(
                        "relationship type `{}` does not connect the member type and `{shared_type}`",
                        relationship.name()
                    )));
                }
            }
            let start = intent
                .source
                .as_ref()
                .map(|raw| anchor(raw, context, "source"))
                .transpose()?;
            RetrievalPlan::SharedNeighbor {
                start,
                member_type,
                shared_type,
                relationship_type: relationship.map(|r| r.name().clone()),
                direction: direction(intent.direction.as_deref())?,
            }
        }
    };
    Ok(ValidatedPlan {
        plan,
        notes,
        reason,
    })
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::{
        EntityType, EntityTypeId, GraphId, OntologyVersion, PropertiesSchema, PropertyDefinition,
        RelationshipTypeId,
    };
    use serde_json::json;

    use super::*;

    fn property(name: &str, property_type: PropertyType) -> PropertyDefinition {
        PropertyDefinition {
            key: PropertyKey::new(name).unwrap(),
            property_type,
            required: false,
        }
    }

    pub(crate) fn fraud_ontology() -> Ontology {
        let graph_id = GraphId::new();
        let entity = |name: &str, properties: Vec<PropertyDefinition>| {
            EntityType::new(
                EntityTypeId::new(),
                graph_id,
                TypeName::new(name).unwrap(),
                PropertiesSchema::new(properties).unwrap(),
            )
        };
        let relationship = |name: &str, source: &str, target: &str| {
            RelationshipType::new(
                RelationshipTypeId::new(),
                graph_id,
                TypeName::new(name).unwrap(),
                TypeName::new(source).unwrap(),
                TypeName::new(target).unwrap(),
                PropertiesSchema::default(),
            )
        };
        Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![
                entity(
                    "Customer",
                    vec![
                        property("customer_id", PropertyType::String),
                        property("age", PropertyType::Integer),
                    ],
                ),
                entity(
                    "Account",
                    vec![property("account_id", PropertyType::String)],
                ),
                entity("Device", vec![property("device_id", PropertyType::String)]),
            ],
            vec![
                relationship("OWNS", "Customer", "Account"),
                relationship("USES", "Customer", "Device"),
            ],
        )
        .unwrap()
    }

    fn intent(value: Value) -> RetrievalIntent {
        serde_json::from_value(value).unwrap()
    }

    fn check(value: Value, selected: &[EntityId]) -> Result<ValidatedPlan, PlanError> {
        check_with(value, selected, &[])
    }

    fn check_with(
        value: Value,
        selected: &[EntityId],
        earlier: &[EntityId],
    ) -> Result<ValidatedPlan, PlanError> {
        let ontology = fraud_ontology();
        let limits = GraphRagLimits::default();
        validate_intent(
            &intent(value),
            &PlanContext {
                ontology: &ontology,
                selected,
                earlier,
                limits: &limits,
            },
        )
    }

    #[test]
    fn a_shared_neighbor_intent_becomes_a_typed_plan() {
        let selected = [EntityId::new()];
        let plan = check(
            json!({"operation": "shared_neighbor", "source": {"ref": "S1"},
                   "entity_type": "Customer", "shared_entity_type": "Device",
                   "relationship_type": "USES", "direction": "outgoing"}),
            &selected,
        )
        .unwrap();
        let RetrievalPlan::SharedNeighbor {
            start:
                Some(Anchor::Entity {
                    id,
                    reference: Some(reference),
                }),
            member_type: Some(member),
            shared_type,
            relationship_type: Some(relationship),
            direction: Direction::Outgoing,
        } = &plan.plan
        else {
            panic!("{plan:?}");
        };
        assert_eq!((id, reference.to_string().as_str()), (&selected[0], "S1"));
        assert_eq!(
            (member.as_str(), shared_type.as_str(), relationship.as_str()),
            ("Customer", "Device", "USES")
        );
        assert_eq!(plan.summary().source.as_deref(), Some("selected entity S1"));
    }

    #[test]
    fn follow_ups_refer_to_entities_of_earlier_questions() {
        let selected = [EntityId::new()];
        let earlier = [EntityId::new(), EntityId::new()];
        let plan = check_with(
            json!({"operation": "find_path", "source": {"ref": "H1"}, "target": {"ref": "H2"}}),
            &selected,
            &earlier,
        )
        .unwrap();
        assert!(
            matches!(plan.plan, RetrievalPlan::FindPath { .. }),
            "{plan:?}"
        );
        assert_eq!(
            plan.summary().source.as_deref(),
            Some("entity H1 from an earlier question")
        );
        assert_eq!(
            plan.summary().target.as_deref(),
            Some("entity H2 from an earlier question")
        );
        // An id the model copied from an earlier question is recognised too.
        let by_id = check_with(
            json!({"operation": "neighbors", "source": {"entity_id": earlier[1].to_string()}}),
            &selected,
            &earlier,
        )
        .unwrap();
        assert!(matches!(
            by_id.plan,
            RetrievalPlan::Neighbors {
                center: Anchor::Entity {
                    reference: Some(KnownRef::Earlier(2)),
                    ..
                },
                ..
            }
        ));
        for reference in ["H3", "H0", "H01", "X1", "h1", "H", "S2"] {
            assert!(
                matches!(
                    check_with(
                        json!({"operation": "neighbors", "source": {"ref": reference}}),
                        &selected,
                        &earlier,
                    ),
                    Err(PlanError::Invalid(_))
                ),
                "{reference}"
            );
        }
    }

    #[test]
    fn match_values_are_typed_by_the_ontology() {
        let plan = check(
            json!({"operation": "neighbors",
                   "source": {"entity_type": "Customer", "match": {"customer_id": "C001", "age": "34"}}}),
            &[],
        )
        .unwrap();
        let RetrievalPlan::Neighbors {
            center: Anchor::Match { filters, .. },
            depth: 1,
            ..
        } = &plan.plan
        else {
            panic!("{plan:?}");
        };
        assert_eq!(
            filters.get(&PropertyKey::new("age").unwrap()),
            Some(&PropertyValue::Integer(34))
        );
        assert_eq!(
            plan.summary().source.as_deref(),
            Some(r#"Customer where age = 34 and customer_id = "C001""#)
        );
    }

    #[test]
    fn invalid_intents_never_become_plans() {
        let selected = [EntityId::new()];
        for value in [
            json!({"operation": "neighbors"}),
            json!({"operation": "neighbors", "source": {"ref": "S2"}}),
            json!({"operation": "neighbors", "source": {"ref": "S1", "entity_id": selected[0].to_string()}}),
            json!({"operation": "neighbors", "source": {"entity_id": "not-a-uuid"}}),
            json!({"operation": "neighbors", "source": {"entity_type": "Merchant"}}),
            json!({"operation": "neighbors", "source": {"entity_type": "Customer", "match": {"email": "x"}}}),
            json!({"operation": "neighbors", "source": {"entity_type": "Customer", "match": {"age": "old"}}}),
            json!({"operation": "neighbors", "source": {"entity_type": "Customer", "match": {"age": null}}}),
            json!({"operation": "neighbors", "source": {"ref": "S1"}, "relationship_type": "PAYS"}),
            json!({"operation": "neighbors", "source": {"ref": "S1"}, "direction": "sideways"}),
            json!({"operation": "find_path", "source": {"ref": "S1"}}),
            json!({"operation": "shared_neighbor", "entity_type": "Customer"}),
            json!({"operation": "shared_neighbor", "entity_type": "Customer", "shared_entity_type": "Device",
                   "relationship_type": "OWNS"}),
        ] {
            assert!(
                matches!(check(value.clone(), &selected), Err(PlanError::Invalid(_))),
                "{value}"
            );
        }
        // Unknown fields (for example a query string) are rejected when decoding.
        assert!(
            serde_json::from_value::<RetrievalIntent>(
                json!({"operation": "neighbors", "gremlin": "g.V().drop()"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<RetrievalIntent>(json!({"operation": "run_query"})).is_err()
        );
    }

    #[test]
    fn depths_beyond_the_limits_are_rejected() {
        let selected = [EntityId::new(), EntityId::new()];
        for value in [
            json!({"operation": "neighbors", "source": {"ref": "S1"}, "max_depth": 4}),
            json!({"operation": "find_path", "source": {"ref": "S1"}, "target": {"ref": "S2"}, "max_depth": 7}),
        ] {
            assert!(matches!(
                check(value, &selected),
                Err(PlanError::LimitExceeded(_))
            ));
        }
        let path = check(
            json!({"operation": "find_path", "source": {"ref": "S1"}, "target": {"ref": "S2"},
                   "relationship_type": "USES"}),
            &selected,
        )
        .unwrap();
        assert!(matches!(
            path.plan,
            RetrievalPlan::FindPath { max_depth: 3, .. }
        ));
        assert_eq!(path.notes.len(), 1, "the ignored filter is reported");
    }

    #[test]
    fn unsupported_questions_keep_a_bounded_reason() {
        let plan = check(
            json!({"operation": "unsupported", "reason": "x".repeat(1_000)}),
            &[],
        )
        .unwrap();
        let RetrievalPlan::Unsupported { reason } = plan.plan else {
            panic!();
        };
        assert!(reason.chars().count() <= MAX_REASON_CHARS + 1);
    }
}
