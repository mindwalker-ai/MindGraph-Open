//! The ingestion mapping contract, its validation, and row transformation.
//!
//! A mapping turns each source record into one or more entities (by alias)
//! and relationships between those aliases. It is validated against a
//! pinned ontology version and the dataset's fields before any graph write.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use mindgraph_domain::{
    Entity, Ontology, Properties, PropertiesSchema, PropertyKey, PropertyType, PropertyValue,
    TypeName,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::profile::looks_numeric;
use crate::record::Record;
use crate::{FieldValue, IngestionError};

/// How each source record maps onto entities and relationships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestionMapping {
    /// Entities produced (or matched) per record, each with a unique alias.
    pub entities: Vec<EntityMapping>,
    /// Relationships between aliases of the same record.
    #[serde(default)]
    pub relationships: Vec<RelationshipMapping>,
}

/// One entity per record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityMapping {
    /// Name used by relationship mappings.
    pub alias: String,
    /// An entity type of the ontology.
    pub entity_type: String,
    /// Whether the entity may be created.
    #[serde(default)]
    pub mode: EntityMode,
    /// Fields that identify the entity (exact match, at least one).
    pub identity: Vec<FieldMapping>,
    /// Properties set when the entity is created.
    #[serde(default)]
    pub properties: Vec<FieldMapping>,
}

/// Entity resolution mode. Exact identity matching only (no fuzzy matching).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityMode {
    /// Reuse the single existing entity with this identity, or create it.
    #[default]
    CreateOrMatch,
    /// The entity must already exist.
    MatchOnly,
}

/// A source field written to an ontology property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldMapping {
    /// The dataset field.
    pub source_field: String,
    /// The ontology property key.
    pub property: String,
}

/// A relationship between two aliases of the same record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipMapping {
    /// A relationship type of the ontology.
    pub relationship_type: String,
    /// Alias of the source entity.
    pub source: String,
    /// Alias of the target entity.
    pub target: String,
    /// Relationship properties.
    #[serde(default)]
    pub properties: Vec<FieldMapping>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedField {
    pub(crate) source_field: String,
    pub(crate) key: PropertyKey,
    pub(crate) property_type: PropertyType,
    pub(crate) required: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedEntity {
    pub(crate) entity_type: TypeName,
    pub(crate) mode: EntityMode,
    pub(crate) identity: Vec<ResolvedField>,
    pub(crate) properties: Vec<ResolvedField>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedRelationship {
    pub(crate) relationship_type: TypeName,
    pub(crate) source: usize,
    pub(crate) target: usize,
    pub(crate) properties: Vec<ResolvedField>,
}

/// A mapping checked against an ontology and a dataset's fields.
#[derive(Debug, Clone)]
pub struct ValidatedMapping {
    pub(crate) entities: Vec<ResolvedEntity>,
    pub(crate) relationships: Vec<ResolvedRelationship>,
}

fn invalid(message: String) -> IngestionError {
    IngestionError::InvalidMapping(message)
}

fn resolve_fields(
    mappings: &[FieldMapping],
    schema: &PropertiesSchema,
    owner: &str,
    source_fields: &BTreeSet<&str>,
    require_required: bool,
) -> Result<Vec<ResolvedField>, IngestionError> {
    let mut resolved = Vec::with_capacity(mappings.len());
    let mut targets = BTreeSet::new();
    for mapping in mappings {
        if !source_fields.contains(mapping.source_field.as_str()) {
            return Err(invalid(format!(
                "source field `{}` does not exist in the dataset",
                mapping.source_field
            )));
        }
        let definition = schema
            .get(&mapping.property)
            .ok_or_else(|| invalid(format!("`{owner}` has no property `{}`", mapping.property)))?;
        if !targets.insert(mapping.property.as_str()) {
            return Err(invalid(format!(
                "property `{}` of `{owner}` is mapped more than once",
                mapping.property
            )));
        }
        resolved.push(ResolvedField {
            source_field: mapping.source_field.clone(),
            key: definition.key.clone(),
            property_type: definition.property_type,
            required: definition.required,
        });
    }
    if require_required
        && let Some(missing) = schema
            .definitions()
            .iter()
            .find(|definition| definition.required && !targets.contains(definition.key.as_str()))
    {
        return Err(invalid(format!(
            "required property `{}` of `{owner}` is not mapped",
            missing.key
        )));
    }
    Ok(resolved)
}

/// Checks a mapping against the pinned ontology and the dataset's fields.
///
/// # Errors
///
/// [`IngestionError::InvalidMapping`] describing the first problem found.
pub fn validate_mapping(
    mapping: &IngestionMapping,
    ontology: &Ontology,
    source_fields: &[String],
) -> Result<ValidatedMapping, IngestionError> {
    if mapping.entities.is_empty() {
        return Err(invalid(
            "at least one entity mapping is required".to_owned(),
        ));
    }
    let fields: BTreeSet<&str> = source_fields.iter().map(String::as_str).collect();
    let mut aliases: BTreeMap<&str, usize> = BTreeMap::new();
    let mut entities = Vec::with_capacity(mapping.entities.len());

    for entity in &mapping.entities {
        let alias = entity.alias.trim();
        if alias.is_empty() {
            return Err(invalid("entity aliases must not be empty".to_owned()));
        }
        if aliases.insert(alias, entities.len()).is_some() {
            return Err(invalid(format!("alias `{alias}` is used more than once")));
        }
        let entity_type = ontology.entity_type(&entity.entity_type).ok_or_else(|| {
            invalid(format!(
                "entity type `{}` is not in the ontology",
                entity.entity_type
            ))
        })?;
        if entity.identity.is_empty() {
            return Err(invalid(format!(
                "entity `{alias}` needs at least one identity field"
            )));
        }
        let schema = entity_type.properties_schema();
        let identity = resolve_fields(
            &entity.identity,
            schema,
            &entity.entity_type,
            &fields,
            false,
        )?;
        let properties = match entity.mode {
            EntityMode::CreateOrMatch => {
                let properties = resolve_fields(
                    &entity.properties,
                    schema,
                    &entity.entity_type,
                    &fields,
                    true,
                )?;
                if let Some(unmapped) = identity
                    .iter()
                    .find(|id| !properties.iter().any(|p| p.key == id.key))
                {
                    return Err(invalid(format!(
                        "identity property `{}` of `{alias}` must also be mapped as a property",
                        unmapped.key
                    )));
                }
                properties
            }
            EntityMode::MatchOnly if !entity.properties.is_empty() => {
                return Err(invalid(format!(
                    "match_only entity `{alias}` cannot set properties"
                )));
            }
            EntityMode::MatchOnly => Vec::new(),
        };
        entities.push(ResolvedEntity {
            entity_type: entity_type.name().clone(),
            mode: entity.mode,
            identity,
            properties,
        });
    }

    let mut relationships = Vec::with_capacity(mapping.relationships.len());
    for relationship in &mapping.relationships {
        let name = &relationship.relationship_type;
        let relationship_type = ontology
            .relationship_type(name)
            .ok_or_else(|| invalid(format!("relationship type `{name}` is not in the ontology")))?;
        let endpoint = |alias: &str, expected: &TypeName, side: &str| {
            let index = *aliases.get(alias).ok_or_else(|| {
                invalid(format!(
                    "relationship `{name}` refers to unknown alias `{alias}`"
                ))
            })?;
            let actual = &entities[index].entity_type;
            if actual != expected {
                return Err(invalid(format!(
                    "relationship `{name}` needs a `{expected}` {side}, but alias `{alias}` is `{actual}`"
                )));
            }
            Ok(index)
        };
        let source = endpoint(
            &relationship.source,
            relationship_type.source_entity_type(),
            "source",
        )?;
        let target = endpoint(
            &relationship.target,
            relationship_type.target_entity_type(),
            "target",
        )?;
        let properties = resolve_fields(
            &relationship.properties,
            relationship_type.properties_schema(),
            name,
            &fields,
            true,
        )?;
        relationships.push(ResolvedRelationship {
            relationship_type: relationship_type.name().clone(),
            source,
            target,
            properties,
        });
    }
    Ok(ValidatedMapping {
        entities,
        relationships,
    })
}

/// Converts a source value to the ontology's property type, or `None` if it
/// cannot be converted. Null is handled by the caller.
pub(crate) fn convert(value: &FieldValue, target: PropertyType) -> Option<PropertyValue> {
    let datetime = |text: &str| {
        DateTime::parse_from_rfc3339(text.trim())
            .ok()
            .map(|at| PropertyValue::DateTime(at.with_timezone(&Utc)))
    };
    match (target, value) {
        (_, FieldValue::Null) => None,
        (PropertyType::String, FieldValue::Raw(text) | FieldValue::Str(text)) => {
            Some(PropertyValue::String(text.clone()))
        }
        (PropertyType::String, FieldValue::Integer(number)) => {
            Some(PropertyValue::String(number.to_string()))
        }
        (PropertyType::String, FieldValue::Float(number)) => {
            Some(PropertyValue::String(number.to_string()))
        }
        (PropertyType::String, FieldValue::Bool(flag)) => {
            Some(PropertyValue::String(flag.to_string()))
        }
        (PropertyType::Integer, FieldValue::Raw(text)) => {
            text.trim().parse().ok().map(PropertyValue::Integer)
        }
        (PropertyType::Integer, FieldValue::Integer(number)) => {
            Some(PropertyValue::Integer(*number))
        }
        (PropertyType::Float, FieldValue::Raw(text)) => {
            let text = text.trim();
            looks_numeric(text)
                .then(|| text.parse::<f64>().ok())
                .flatten()
                .filter(|number| number.is_finite())
                .map(PropertyValue::Float)
        }
        #[allow(clippy::cast_precision_loss)] // Documented: integers widen to floats.
        (PropertyType::Float, FieldValue::Integer(number)) => {
            Some(PropertyValue::Float(*number as f64))
        }
        (PropertyType::Float, FieldValue::Float(number)) => Some(PropertyValue::Float(*number)),
        (PropertyType::Boolean, FieldValue::Raw(text)) => {
            let text = text.trim();
            if text.eq_ignore_ascii_case("true") {
                Some(PropertyValue::Boolean(true))
            } else if text.eq_ignore_ascii_case("false") {
                Some(PropertyValue::Boolean(false))
            } else {
                None
            }
        }
        (PropertyType::Boolean, FieldValue::Bool(flag)) => Some(PropertyValue::Boolean(*flag)),
        (PropertyType::DateTime, FieldValue::Raw(text) | FieldValue::Str(text)) => datetime(text),
        (PropertyType::StringArray, FieldValue::StringArray(items)) => {
            Some(PropertyValue::StringArray(items.clone()))
        }
        _ => None,
    }
}

/// An entity planned from one record.
#[derive(Debug, Clone)]
pub(crate) struct PlannedEntity {
    /// `type` + identity values, canonical; the job-scoped cache key.
    pub(crate) identity_key: String,
    pub(crate) identity: Vec<(PropertyKey, PropertyValue)>,
    pub(crate) properties: Properties,
}

/// Everything one record produces.
#[derive(Debug, Clone)]
pub(crate) struct RowPlan {
    pub(crate) record: u64,
    pub(crate) entities: Vec<PlannedEntity>,
    pub(crate) relationships: Vec<Properties>,
}

fn read_field(
    record: &Record,
    number: u64,
    field: &ResolvedField,
) -> Result<Option<PropertyValue>, IngestionError> {
    match record.get(&field.source_field) {
        None | Some(FieldValue::Null) => Ok(None),
        Some(value) => convert(value, field.property_type)
            .map(Some)
            .ok_or_else(|| IngestionError::TypeConversion {
                record: number,
                field: field.source_field.clone(),
                expected: field.property_type,
            }),
    }
}

fn properties_of(
    record: &Record,
    number: u64,
    fields: &[ResolvedField],
) -> Result<Properties, IngestionError> {
    let mut properties = Properties::new();
    for field in fields {
        match read_field(record, number, field)? {
            Some(value) => {
                properties.insert(field.key.clone(), value).map_err(|_| {
                    IngestionError::TypeConversion {
                        record: number,
                        field: field.source_field.clone(),
                        expected: field.property_type,
                    }
                })?;
            }
            None if field.required => {
                return Err(IngestionError::MissingField {
                    record: number,
                    field: field.source_field.clone(),
                });
            }
            None => {}
        }
    }
    Ok(properties)
}

/// The canonical, job-scoped cache key of an identity: the type plus the
/// identity values in mapping order.
pub(crate) fn identity_key(
    entity_type: &TypeName,
    identity: &[(PropertyKey, PropertyValue)],
) -> String {
    let canonical: Vec<Value> = identity
        .iter()
        .map(|(key, value)| json!([key.as_str(), value.to_json()]))
        .collect();
    format!("{entity_type}\u{1f}{}", Value::Array(canonical))
}

/// The identity key of a stored entity under `mapping`, or `None` if the
/// entity lacks an identity property.
pub(crate) fn entity_identity_key(mapping: &ResolvedEntity, entity: &Entity) -> Option<String> {
    let identity = mapping
        .identity
        .iter()
        .map(|field| {
            entity
                .properties()
                .get(field.key.as_str())
                .map(|value| (field.key.clone(), value.clone()))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(identity_key(&mapping.entity_type, &identity))
}

/// Transforms one record according to a validated mapping.
pub(crate) fn plan_row(
    mapping: &ValidatedMapping,
    number: u64,
    record: &Record,
) -> Result<RowPlan, IngestionError> {
    let mut entities = Vec::with_capacity(mapping.entities.len());
    for entity in &mapping.entities {
        let mut identity = Vec::with_capacity(entity.identity.len());
        for field in &entity.identity {
            let value =
                read_field(record, number, field)?.ok_or_else(|| IngestionError::MissingField {
                    record: number,
                    field: field.source_field.clone(),
                })?;
            identity.push((field.key.clone(), value));
        }
        entities.push(PlannedEntity {
            identity_key: identity_key(&entity.entity_type, &identity),
            identity,
            properties: properties_of(record, number, &entity.properties)?,
        });
    }
    let relationships = mapping
        .relationships
        .iter()
        .map(|relationship| properties_of(record, number, &relationship.properties))
        .collect::<Result<_, _>>()?;
    Ok(RowPlan {
        record: number,
        entities,
        relationships,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use mindgraph_domain::{
        EntityType, EntityTypeId, GraphId, OntologyVersion, PropertyDefinition, RelationshipType,
        RelationshipTypeId,
    };

    use super::*;

    pub(crate) fn fraud_ontology(graph_id: GraphId) -> Ontology {
        let key = |k: &str| PropertyKey::new(k).unwrap();
        let name = |n: &str| TypeName::new(n).unwrap();
        let customer = EntityType::new(
            EntityTypeId::new(),
            graph_id,
            name("Customer"),
            PropertiesSchema::new(vec![
                PropertyDefinition::required(key("customer_id"), PropertyType::String),
                PropertyDefinition::optional(key("name"), PropertyType::String),
                PropertyDefinition::optional(key("age"), PropertyType::Integer),
            ])
            .unwrap(),
        );
        let account = EntityType::new(
            EntityTypeId::new(),
            graph_id,
            name("Account"),
            PropertiesSchema::new(vec![
                PropertyDefinition::required(key("account_id"), PropertyType::String),
                PropertyDefinition::optional(key("balance"), PropertyType::Float),
            ])
            .unwrap(),
        );
        let owns = RelationshipType::new(
            RelationshipTypeId::new(),
            graph_id,
            name("OWNS"),
            name("Customer"),
            name("Account"),
            PropertiesSchema::new(vec![PropertyDefinition::optional(
                key("since"),
                PropertyType::DateTime,
            )])
            .unwrap(),
        );
        Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![customer, account],
            vec![owns],
        )
        .unwrap()
    }

    pub(crate) fn field(source: &str, property: &str) -> FieldMapping {
        FieldMapping {
            source_field: source.into(),
            property: property.into(),
        }
    }

    pub(crate) fn customer_mapping() -> EntityMapping {
        EntityMapping {
            alias: "customer".into(),
            entity_type: "Customer".into(),
            mode: EntityMode::CreateOrMatch,
            identity: vec![field("customer_id", "customer_id")],
            properties: vec![field("customer_id", "customer_id"), field("name", "name")],
        }
    }

    pub(crate) fn account_mapping() -> EntityMapping {
        EntityMapping {
            alias: "account".into(),
            entity_type: "Account".into(),
            mode: EntityMode::CreateOrMatch,
            identity: vec![field("account_id", "account_id")],
            properties: vec![
                field("account_id", "account_id"),
                field("balance", "balance"),
            ],
        }
    }

    pub(crate) fn owns(source: &str, target: &str) -> RelationshipMapping {
        RelationshipMapping {
            relationship_type: "OWNS".into(),
            source: source.into(),
            target: target.into(),
            properties: vec![],
        }
    }

    fn fields() -> Vec<String> {
        ["customer_id", "name", "account_id", "balance", "age"]
            .map(String::from)
            .to_vec()
    }

    fn check(mapping: &IngestionMapping) -> Result<ValidatedMapping, IngestionError> {
        validate_mapping(mapping, &fraud_ontology(GraphId::new()), &fields())
    }

    fn err(mapping: &IngestionMapping) -> String {
        check(mapping).unwrap_err().to_string()
    }

    #[test]
    fn valid_multi_entity_mapping() {
        let mapping = IngestionMapping {
            entities: vec![customer_mapping(), account_mapping()],
            relationships: vec![owns("customer", "account")],
        };
        let validated = check(&mapping).unwrap();
        assert_eq!(validated.entities.len(), 2);
        assert_eq!(validated.relationships[0].source, 0);
    }

    #[test]
    fn mapping_json_contract() {
        let json = r#"{"entities":[{"alias":"c","entity_type":"Customer","identity":[{"source_field":"customer_id","property":"customer_id"}],"properties":[{"source_field":"customer_id","property":"customer_id"}]}]}"#;
        let mapping: IngestionMapping = serde_json::from_str(json).unwrap();
        assert_eq!(
            mapping.entities[0].mode,
            EntityMode::CreateOrMatch,
            "default mode"
        );
        assert!(mapping.relationships.is_empty());
        assert!(serde_json::from_str::<IngestionMapping>(r#"{"entities":[],"extra":1}"#).is_err());
    }

    #[test]
    fn invalid_mappings_are_rejected() {
        let base = || IngestionMapping {
            entities: vec![customer_mapping(), account_mapping()],
            relationships: vec![owns("customer", "account")],
        };
        let mut m = base();
        m.entities[0].properties.push(field("nope", "name"));
        assert!(err(&m).contains("source field `nope`"));

        let mut m = base();
        m.entities[0].entity_type = "Merchant".into();
        assert!(err(&m).contains("entity type `Merchant`"));

        let mut m = base();
        m.entities[0].properties.push(field("age", "height"));
        assert!(err(&m).contains("no property `height`"));

        let mut m = base();
        m.entities[1].alias = "customer".into();
        assert!(err(&m).contains("used more than once"));

        let mut m = base();
        m.entities[0].identity.clear();
        assert!(err(&m).contains("at least one identity"));

        let mut m = base();
        m.relationships[0].relationship_type = "KNOWS".into();
        assert!(err(&m).contains("relationship type `KNOWS`"));

        let mut m = base();
        m.relationships[0].target = "merchant".into();
        assert!(err(&m).contains("unknown alias `merchant`"));

        let mut m = base();
        m.relationships[0] = owns("account", "customer");
        assert!(err(&m).contains("needs a `Customer` source"));

        let mut m = base();
        m.entities[0]
            .properties
            .retain(|p| p.property != "customer_id");
        assert!(err(&m).contains("required property `customer_id`"));

        let mut m = base();
        m.entities[0].identity = vec![field("name", "name")];
        m.entities[0].properties = vec![field("customer_id", "customer_id")];
        assert!(err(&m).contains("identity property `name`"));

        let mut m = base();
        m.entities[1].mode = EntityMode::MatchOnly;
        assert!(err(&m).contains("cannot set properties"));

        assert!(
            err(&IngestionMapping {
                entities: vec![],
                relationships: vec![]
            })
            .contains("at least one")
        );
    }

    #[test]
    fn conversion_uses_the_ontology_type() {
        let raw = |t: &str| FieldValue::Raw(t.into());
        assert_eq!(
            convert(&raw("00123"), PropertyType::String),
            Some(PropertyValue::String("00123".into()))
        );
        assert_eq!(
            convert(&raw("42"), PropertyType::Integer),
            Some(PropertyValue::Integer(42))
        );
        assert_eq!(convert(&raw("4.5"), PropertyType::Integer), None);
        assert_eq!(
            convert(&raw("4.5"), PropertyType::Float),
            Some(PropertyValue::Float(4.5))
        );
        assert_eq!(convert(&raw("NaN"), PropertyType::Float), None);
        assert_eq!(
            convert(&raw("False"), PropertyType::Boolean),
            Some(PropertyValue::Boolean(false))
        );
        assert_eq!(convert(&raw("yes"), PropertyType::Boolean), None);
        assert!(convert(&raw("2026-11-03T08:30:00+07:00"), PropertyType::DateTime).is_some());
        assert_eq!(
            convert(&raw("a,b"), PropertyType::StringArray),
            None,
            "no CSV array splitting"
        );
        assert_eq!(
            convert(&FieldValue::Integer(3), PropertyType::Float),
            Some(PropertyValue::Float(3.0))
        );
        assert_eq!(
            convert(&FieldValue::Str("12".into()), PropertyType::Integer),
            None
        );
        assert_eq!(
            convert(
                &FieldValue::StringArray(vec!["a".into()]),
                PropertyType::StringArray
            ),
            Some(PropertyValue::StringArray(vec!["a".into()]))
        );
    }

    #[test]
    fn rows_fail_on_missing_or_unconvertible_values() {
        let mapping = check(&IngestionMapping {
            entities: vec![customer_mapping()],
            relationships: vec![],
        })
        .unwrap();
        let mut record = Record::new();
        record.insert("customer_id".into(), FieldValue::Null);
        assert!(matches!(
            plan_row(&mapping, 3, &record),
            Err(IngestionError::MissingField { record: 3, .. })
        ));
        let mut typed = check(&IngestionMapping {
            entities: vec![EntityMapping {
                properties: vec![field("customer_id", "customer_id"), field("age", "age")],
                ..customer_mapping()
            }],
            relationships: vec![],
        })
        .unwrap();
        let mut record = Record::new();
        record.insert("customer_id".into(), FieldValue::Raw("C1".into()));
        record.insert("age".into(), FieldValue::Raw("old".into()));
        let error = plan_row(&typed, 7, &record).unwrap_err().to_string();
        assert!(
            error.contains("record 7") && error.contains("`age`"),
            "{error}"
        );
        assert!(!error.contains("old"), "values are never echoed");
        record.insert("age".into(), FieldValue::Null);
        typed = typed.clone();
        let plan = plan_row(&typed, 7, &record).unwrap();
        assert!(
            !plan.entities[0].properties.contains_key("age"),
            "optional nulls are omitted"
        );
    }
}
