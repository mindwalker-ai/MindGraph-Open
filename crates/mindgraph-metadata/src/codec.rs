//! Persistence codec for ontology snapshots (`ontology_versions.schema_json`).
//!
//! The JSON document is an explicit, versioned format owned by this adapter.
//! It is not a dump of the domain's private fields. Decoding rebuilds the
//! domain objects through their validated constructors, so a stored ontology
//! that breaks a domain rule is reported as corrupt instead of loaded.
//!
//! ```json
//! {
//!   "format": 1,
//!   "entity_types": [
//!     {"id": "019…", "name": "Customer", "display_name": "Customer", "description": null,
//!      "properties": [{"key": "name", "type": "string", "required": true}]}
//!   ],
//!   "relationship_types": [
//!     {"id": "019…", "name": "OWNS", "source_entity_type": "Customer",
//!      "target_entity_type": "Account", "properties": []}
//!   ]
//! }
//! ```
//!
//! The graph id and version are stored in their own columns, not in the
//! document.

use mindgraph_core::MetadataStoreError;
use mindgraph_domain::{
    EntityType, GraphId, Ontology, OntologyVersion, PropertiesSchema, PropertyDefinition,
    PropertyKey, PropertyType, RelationshipType, TypeName,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OntologyDocument {
    format: u32,
    entity_types: Vec<EntityTypeDocument>,
    relationship_types: Vec<RelationshipTypeDocument>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityTypeDocument {
    id: String,
    name: String,
    display_name: String,
    description: Option<String>,
    properties: Vec<PropertyDocument>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelationshipTypeDocument {
    id: String,
    name: String,
    source_entity_type: String,
    target_entity_type: String,
    properties: Vec<PropertyDocument>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PropertyDocument {
    key: String,
    #[serde(rename = "type")]
    property_type: PropertyType,
    required: bool,
}

fn properties_document(schema: &PropertiesSchema) -> Vec<PropertyDocument> {
    schema
        .definitions()
        .iter()
        .map(|definition| PropertyDocument {
            key: definition.key.to_string(),
            property_type: definition.property_type,
            required: definition.required,
        })
        .collect()
}

/// Encodes an ontology's types as the `schema_json` document.
pub(crate) fn encode(ontology: &Ontology) -> Result<Value, MetadataStoreError> {
    let document = OntologyDocument {
        format: FORMAT_VERSION,
        entity_types: ontology
            .entity_types()
            .iter()
            .map(|entity_type| EntityTypeDocument {
                id: entity_type.id().to_string(),
                name: entity_type.name().to_string(),
                display_name: entity_type.display_name().to_owned(),
                description: entity_type.description().map(str::to_owned),
                properties: properties_document(entity_type.properties_schema()),
            })
            .collect(),
        relationship_types: ontology
            .relationship_types()
            .iter()
            .map(|relationship_type| RelationshipTypeDocument {
                id: relationship_type.id().to_string(),
                name: relationship_type.name().to_string(),
                source_entity_type: relationship_type.source_entity_type().to_string(),
                target_entity_type: relationship_type.target_entity_type().to_string(),
                properties: properties_document(relationship_type.properties_schema()),
            })
            .collect(),
    };
    serde_json::to_value(document).map_err(|err| MetadataStoreError::Serialization(err.to_string()))
}

/// Rebuilds an ontology from a stored document.
///
/// Every failure is [`MetadataStoreError::CorruptData`].
pub(crate) fn decode(
    graph_id: GraphId,
    version: OntologyVersion,
    value: Value,
) -> Result<Ontology, MetadataStoreError> {
    let corrupt = |reason: String| {
        MetadataStoreError::CorruptData(format!("ontology {version} of graph {graph_id}: {reason}"))
    };
    let document: OntologyDocument =
        serde_json::from_value(value).map_err(|err| corrupt(err.to_string()))?;
    if document.format != FORMAT_VERSION {
        return Err(corrupt(format!("unsupported format {}", document.format)));
    }
    let domain = |err: mindgraph_domain::DomainError| corrupt(err.to_string());

    let schema =
        |properties: Vec<PropertyDocument>| -> Result<PropertiesSchema, MetadataStoreError> {
            let definitions = properties
                .into_iter()
                .map(|property| {
                    Ok(PropertyDefinition {
                        key: PropertyKey::new(property.key).map_err(domain)?,
                        property_type: property.property_type,
                        required: property.required,
                    })
                })
                .collect::<Result<Vec<_>, MetadataStoreError>>()?;
            PropertiesSchema::new(definitions).map_err(domain)
        };

    let mut entity_types = Vec::with_capacity(document.entity_types.len());
    for entity_type in document.entity_types {
        let mut decoded = EntityType::new(
            entity_type.id.parse().map_err(domain)?,
            graph_id,
            TypeName::new(entity_type.name).map_err(domain)?,
            schema(entity_type.properties)?,
        )
        .with_display_name(&entity_type.display_name)
        .map_err(domain)?;
        if let Some(description) = entity_type.description {
            decoded = decoded.with_description(&description).map_err(domain)?;
        }
        entity_types.push(decoded);
    }

    let mut relationship_types = Vec::with_capacity(document.relationship_types.len());
    for relationship_type in document.relationship_types {
        relationship_types.push(RelationshipType::new(
            relationship_type.id.parse().map_err(domain)?,
            graph_id,
            TypeName::new(relationship_type.name).map_err(domain)?,
            TypeName::new(relationship_type.source_entity_type).map_err(domain)?,
            TypeName::new(relationship_type.target_entity_type).map_err(domain)?,
            schema(relationship_type.properties)?,
        ));
    }

    Ontology::new(graph_id, version, entity_types, relationship_types).map_err(domain)
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::{EntityTypeId, RelationshipTypeId};
    use serde_json::json;

    use super::*;

    fn key(value: &str) -> PropertyKey {
        PropertyKey::new(value).unwrap()
    }

    fn name(value: &str) -> TypeName {
        TypeName::new(value).unwrap()
    }

    /// Every property type, required and optional, display names and
    /// descriptions, and relationship types.
    fn rich_ontology(graph_id: GraphId) -> Ontology {
        let customer_schema = PropertiesSchema::new(vec![
            PropertyDefinition::required(key("customer_id"), PropertyType::String),
            PropertyDefinition::optional(key("age"), PropertyType::Integer),
            PropertyDefinition::optional(key("risk_score"), PropertyType::Float),
            PropertyDefinition::optional(key("verified"), PropertyType::Boolean),
            PropertyDefinition::required(key("joined_at"), PropertyType::DateTime),
            PropertyDefinition::optional(key("emails"), PropertyType::StringArray),
        ])
        .unwrap();
        let customer = EntityType::new(
            EntityTypeId::new(),
            graph_id,
            name("Customer"),
            customer_schema,
        )
        .with_display_name("Bank Customer")
        .unwrap()
        .with_description("A retail banking customer")
        .unwrap();
        let account = EntityType::new(
            EntityTypeId::new(),
            graph_id,
            name("Account"),
            PropertiesSchema::default(),
        );
        let owns_schema = PropertiesSchema::new(vec![PropertyDefinition::required(
            key("since"),
            PropertyType::DateTime,
        )])
        .unwrap();
        let owns = RelationshipType::new(
            RelationshipTypeId::new(),
            graph_id,
            name("OWNS"),
            name("Customer"),
            name("Account"),
            owns_schema,
        );
        let transfer = RelationshipType::new(
            RelationshipTypeId::new(),
            graph_id,
            name("TRANSFERRED_TO"),
            name("Account"),
            name("Account"),
            PropertiesSchema::default(),
        );
        Ontology::new(
            graph_id,
            OntologyVersion::new(3).unwrap(),
            vec![customer, account],
            vec![owns, transfer],
        )
        .unwrap()
    }

    #[test]
    fn rich_ontologies_round_trip() {
        let graph_id = GraphId::new();
        let ontology = rich_ontology(graph_id);
        let encoded = encode(&ontology).unwrap();
        let decoded = decode(graph_id, ontology.version(), encoded).unwrap();
        assert_eq!(decoded, ontology);
    }

    #[test]
    fn empty_ontologies_round_trip() {
        let graph_id = GraphId::new();
        let ontology = Ontology::new(graph_id, OntologyVersion::INITIAL, vec![], vec![]).unwrap();
        let encoded = encode(&ontology).unwrap();
        assert_eq!(
            encoded,
            json!({"format": 1, "entity_types": [], "relationship_types": []})
        );
        assert_eq!(
            decode(graph_id, OntologyVersion::INITIAL, encoded).unwrap(),
            ontology
        );
    }

    #[test]
    fn encoding_preserves_order_and_property_flags() {
        let ontology = rich_ontology(GraphId::new());
        let encoded = encode(&ontology).unwrap();
        let customer = &encoded["entity_types"][0];
        assert_eq!(customer["name"], "Customer");
        assert_eq!(customer["display_name"], "Bank Customer");
        let keys: Vec<&str> = customer["properties"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["key"].as_str().unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                "customer_id",
                "age",
                "risk_score",
                "verified",
                "joined_at",
                "emails"
            ]
        );
        assert_eq!(
            customer["properties"][0],
            json!({"key": "customer_id", "type": "string", "required": true})
        );
        assert_eq!(customer["properties"][5]["type"], "string_array");
        assert_eq!(customer["properties"][4]["type"], "datetime");
        assert_eq!(encoded["relationship_types"][1]["name"], "TRANSFERRED_TO");
    }

    #[test]
    fn corrupt_documents_are_corrupt_data() {
        let graph_id = GraphId::new();
        let mut unknown_endpoint = encode(&rich_ontology(graph_id)).unwrap();
        unknown_endpoint["relationship_types"][0]["target_entity_type"] = json!("Merchant");
        let mut bad_type = encode(&rich_ontology(graph_id)).unwrap();
        bad_type["entity_types"][0]["properties"][0]["type"] = json!("decimal");
        let mut bad_id = encode(&rich_ontology(graph_id)).unwrap();
        bad_id["entity_types"][0]["id"] = json!("not-a-uuid");

        let cases = [
            json!(null),
            json!([]),
            json!({"format": 2, "entity_types": [], "relationship_types": []}),
            json!({"format": 1, "entity_types": []}),
            json!({"format": 1, "entity_types": [], "relationship_types": [], "extra": 1}),
            unknown_endpoint,
            bad_type,
            bad_id,
        ];
        for case in cases {
            assert!(
                matches!(
                    decode(graph_id, OntologyVersion::INITIAL, case.clone()),
                    Err(MetadataStoreError::CorruptData(_))
                ),
                "{case}"
            );
        }
    }
}
