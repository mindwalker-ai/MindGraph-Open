//! Mapping between domain objects and HugeGraph's generic physical model.
//!
//! ```text
//! Entity        ──► vertex  label  mindgraph_entity
//!                           id     "{graph_id}:{entity_id}"
//!                           mg_graph_id, mg_type, mg_properties
//! Relationship  ──► edge    label  mindgraph_relationship   (sort key mg_id)
//!                           mg_id, mg_graph_id, mg_type, mg_properties
//! ```
//!
//! The vertex id is prefixed with the graph id, so identity is exactly
//! `GraphId + EntityId` and HugeGraph can never connect or overwrite
//! vertices of different MindGraph graphs.
//!
//! `mg_properties` holds the domain properties in a typed JSON encoding.
//! Plain JSON (`Properties::to_json`) would be ambiguous without an
//! ontology (a datetime and a string look the same), so each value carries
//! its type:
//!
//! ```json
//! {"version":1,"properties":{"joined_at":{"type":"datetime","value":"2026-11-03T08:30:00Z"}}}
//! ```

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use mindgraph_domain::{
    Entity, EntityId, GraphId, Properties, PropertyKey, PropertyValue, Relationship,
    RelationshipId, TypeName,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::dto::{Edge, NewEdge, NewVertex, Vertex};
use crate::GraphStoreError;

pub(crate) const VERTEX_LABEL: &str = "mindgraph_entity";
pub(crate) const EDGE_LABEL: &str = "mindgraph_relationship";
pub(crate) const PROP_GRAPH_ID: &str = "mg_graph_id";
pub(crate) const PROP_TYPE: &str = "mg_type";
pub(crate) const PROP_ID: &str = "mg_id";
pub(crate) const PROP_PROPERTIES: &str = "mg_properties";

/// Version of the `mg_properties` encoding.
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredProperties {
    version: u32,
    properties: BTreeMap<String, StoredValue>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum StoredValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Datetime(String),
    StringArray(Vec<String>),
}

fn corrupt(message: impl Into<String>) -> GraphStoreError {
    GraphStoreError::CorruptData(message.into())
}

/// The HugeGraph vertex id of an entity.
pub(crate) fn vertex_id(graph_id: GraphId, entity_id: EntityId) -> String {
    format!("{graph_id}:{entity_id}")
}

/// Splits a HugeGraph vertex id back into its graph and entity ids.
pub(crate) fn parse_vertex_id(raw: &Value) -> Result<(GraphId, EntityId), GraphStoreError> {
    let text = raw
        .as_str()
        .ok_or_else(|| corrupt(format!("vertex id {raw} is not a string")))?;
    let (graph, entity) = text
        .split_once(':')
        .ok_or_else(|| corrupt(format!("vertex id `{text}` is not `graph_id:entity_id`")))?;
    let graph_id = graph
        .parse()
        .map_err(|err| corrupt(format!("vertex id `{text}`: {err}")))?;
    let entity_id = entity
        .parse()
        .map_err(|err| corrupt(format!("vertex id `{text}`: {err}")))?;
    Ok((graph_id, entity_id))
}

/// Encodes domain properties for the `mg_properties` field.
pub(crate) fn encode_properties(properties: &Properties) -> Result<String, GraphStoreError> {
    let mut stored = BTreeMap::new();
    for (key, value) in properties.iter() {
        let value = match value {
            PropertyValue::String(text) => StoredValue::String(text.clone()),
            PropertyValue::Integer(number) => StoredValue::Integer(*number),
            PropertyValue::Float(number) if number.is_finite() => StoredValue::Float(*number),
            PropertyValue::Float(_) => {
                return Err(GraphStoreError::Serialization(format!(
                    "property `{key}` is not a finite float"
                )));
            }
            PropertyValue::Boolean(flag) => StoredValue::Boolean(*flag),
            PropertyValue::DateTime(at) => {
                StoredValue::Datetime(at.to_rfc3339_opts(SecondsFormat::AutoSi, true))
            }
            PropertyValue::StringArray(items) => StoredValue::StringArray(items.clone()),
        };
        stored.insert(key.to_string(), value);
    }
    serde_json::to_string(&StoredProperties {
        version: FORMAT_VERSION,
        properties: stored,
    })
    .map_err(|err| GraphStoreError::Serialization(err.to_string()))
}

/// Decodes an `mg_properties` field. Any malformed input is `CorruptData`.
pub(crate) fn decode_properties(raw: &str) -> Result<Properties, GraphStoreError> {
    let stored: StoredProperties = serde_json::from_str(raw)
        .map_err(|err| corrupt(format!("malformed {PROP_PROPERTIES}: {err}")))?;
    if stored.version != FORMAT_VERSION {
        return Err(corrupt(format!(
            "unsupported {PROP_PROPERTIES} version {}",
            stored.version
        )));
    }
    let mut properties = Properties::new();
    for (key, value) in stored.properties {
        let key = PropertyKey::new(key).map_err(|err| corrupt(err.to_string()))?;
        let value = match value {
            StoredValue::String(text) => PropertyValue::String(text),
            StoredValue::Integer(number) => PropertyValue::Integer(number),
            StoredValue::Float(number) => PropertyValue::Float(number),
            StoredValue::Boolean(flag) => PropertyValue::Boolean(flag),
            StoredValue::Datetime(text) => DateTime::parse_from_rfc3339(&text)
                .map(|at| PropertyValue::DateTime(at.with_timezone(&Utc)))
                .map_err(|err| corrupt(format!("property `{key}` has invalid datetime: {err}")))?,
            StoredValue::StringArray(items) => PropertyValue::StringArray(items),
        };
        properties
            .insert(key, value)
            .map_err(|err| corrupt(err.to_string()))?;
    }
    Ok(properties)
}

fn required_str<'a>(
    properties: &'a Map<String, Value>,
    key: &str,
    owner: &str,
) -> Result<&'a str, GraphStoreError> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt(format!("{owner} has no string property `{key}`")))
}

/// The HugeGraph representation of an entity.
pub(crate) fn new_vertex(entity: &Entity) -> Result<NewVertex, GraphStoreError> {
    let mut properties = Map::new();
    properties.insert(PROP_GRAPH_ID.into(), entity.graph_id().to_string().into());
    properties.insert(PROP_TYPE.into(), entity.entity_type().to_string().into());
    properties.insert(
        PROP_PROPERTIES.into(),
        encode_properties(entity.properties())?.into(),
    );
    Ok(NewVertex {
        id: vertex_id(entity.graph_id(), entity.id()),
        label: VERTEX_LABEL,
        properties,
    })
}

/// Rebuilds an entity from a HugeGraph vertex.
pub(crate) fn entity_from_vertex(vertex: &Vertex) -> Result<Entity, GraphStoreError> {
    if vertex.label != VERTEX_LABEL {
        return Err(corrupt(format!(
            "vertex {} has label `{}`, expected `{VERTEX_LABEL}`",
            vertex.id, vertex.label
        )));
    }
    let (graph_id, entity_id) = parse_vertex_id(&vertex.id)?;
    let owner = format!("vertex {}", vertex.id);
    let stored_graph = required_str(&vertex.properties, PROP_GRAPH_ID, &owner)?;
    if stored_graph != graph_id.to_string() {
        return Err(corrupt(format!(
            "{owner} stores {PROP_GRAPH_ID} `{stored_graph}`, which does not match its id"
        )));
    }
    let entity_type = TypeName::new(required_str(&vertex.properties, PROP_TYPE, &owner)?)
        .map_err(|err| corrupt(format!("{owner}: {err}")))?;
    let properties = decode_properties(required_str(&vertex.properties, PROP_PROPERTIES, &owner)?)?;
    Ok(Entity::new(entity_id, graph_id, entity_type, properties))
}

/// The HugeGraph representation of a relationship.
pub(crate) fn new_edge(relationship: &Relationship) -> Result<NewEdge, GraphStoreError> {
    let graph_id = relationship.graph_id();
    let mut properties = Map::new();
    properties.insert(PROP_ID.into(), relationship.id().to_string().into());
    properties.insert(PROP_GRAPH_ID.into(), graph_id.to_string().into());
    properties.insert(
        PROP_TYPE.into(),
        relationship.relationship_type().to_string().into(),
    );
    properties.insert(
        PROP_PROPERTIES.into(),
        encode_properties(relationship.properties())?.into(),
    );
    Ok(NewEdge {
        label: EDGE_LABEL,
        out_v: vertex_id(graph_id, relationship.source_id()),
        out_v_label: VERTEX_LABEL,
        in_v: vertex_id(graph_id, relationship.target_id()),
        in_v_label: VERTEX_LABEL,
        properties,
    })
}

/// Rebuilds a relationship from a HugeGraph edge.
pub(crate) fn relationship_from_edge(edge: &Edge) -> Result<Relationship, GraphStoreError> {
    if edge.label != EDGE_LABEL {
        return Err(corrupt(format!(
            "edge has label `{}`, expected `{EDGE_LABEL}`",
            edge.label
        )));
    }
    let id_text = required_str(&edge.properties, PROP_ID, "edge")?;
    let relationship_id: RelationshipId = id_text
        .parse()
        .map_err(|err| corrupt(format!("edge {PROP_ID}: {err}")))?;
    let owner = format!("relationship {relationship_id}");
    let graph_id: GraphId = required_str(&edge.properties, PROP_GRAPH_ID, &owner)?
        .parse()
        .map_err(|err| corrupt(format!("{owner}: {err}")))?;
    let (source_graph, source_id) = parse_vertex_id(&edge.out_v)?;
    let (target_graph, target_id) = parse_vertex_id(&edge.in_v)?;
    if source_graph != graph_id || target_graph != graph_id {
        return Err(corrupt(format!(
            "{owner} connects vertices outside its graph {graph_id}"
        )));
    }
    let relationship_type = TypeName::new(required_str(&edge.properties, PROP_TYPE, &owner)?)
        .map_err(|err| corrupt(format!("{owner}: {err}")))?;
    let properties = decode_properties(required_str(&edge.properties, PROP_PROPERTIES, &owner)?)?;
    Ok(Relationship::new(
        relationship_id,
        graph_id,
        relationship_type,
        source_id,
        target_id,
        properties,
    ))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn key(value: &str) -> PropertyKey {
        PropertyKey::new(value).unwrap()
    }

    fn round_trip(value: PropertyValue) {
        let mut properties = Properties::new();
        properties.insert(key("value"), value).unwrap();
        let encoded = encode_properties(&properties).unwrap();
        assert_eq!(
            decode_properties(&encoded).unwrap(),
            properties,
            "{encoded}"
        );
    }

    #[test]
    fn every_property_type_round_trips() {
        round_trip(PropertyValue::String("Ikhwan Nasution".into()));
        round_trip(PropertyValue::String(String::new()));
        round_trip(PropertyValue::Integer(i64::MIN));
        round_trip(PropertyValue::Integer(i64::MAX));
        round_trip(PropertyValue::Float(0.1 + 0.2));
        round_trip(PropertyValue::Float(-1.5e-300));
        round_trip(PropertyValue::Float(42.0));
        round_trip(PropertyValue::Boolean(true));
        round_trip(PropertyValue::DateTime(
            Utc.with_ymd_and_hms(2026, 11, 3, 8, 30, 0).unwrap(),
        ));
        round_trip(PropertyValue::DateTime(
            DateTime::parse_from_rfc3339("2026-11-03T08:30:00.123456789Z")
                .unwrap()
                .with_timezone(&Utc),
        ));
        round_trip(PropertyValue::StringArray(vec![
            "a@example.com".into(),
            "b@example.com".into(),
        ]));
        round_trip(PropertyValue::StringArray(vec![]));
    }

    #[test]
    fn datetime_and_string_stay_distinct() {
        let mut properties = Properties::new();
        properties
            .insert(key("as_text"), "2026-11-03T08:30:00Z")
            .unwrap();
        properties
            .insert(
                key("as_time"),
                Utc.with_ymd_and_hms(2026, 11, 3, 8, 30, 0).unwrap(),
            )
            .unwrap();
        let decoded = decode_properties(&encode_properties(&properties).unwrap()).unwrap();
        assert!(matches!(
            decoded.get("as_text"),
            Some(PropertyValue::String(_))
        ));
        assert!(matches!(
            decoded.get("as_time"),
            Some(PropertyValue::DateTime(_))
        ));
    }

    #[test]
    fn encoding_is_deterministic_and_typed() {
        let mut properties = Properties::new();
        properties.insert(key("name"), "Ikhwan").unwrap();
        properties.insert(key("age"), 34_i64).unwrap();
        properties.insert(key("score"), 0.91).unwrap();
        let encoded = encode_properties(&properties).unwrap();
        assert_eq!(
            encoded,
            r#"{"version":1,"properties":{"age":{"type":"integer","value":34},"name":{"type":"string","value":"Ikhwan"},"score":{"type":"float","value":0.91}}}"#
        );
        assert_eq!(encode_properties(&properties).unwrap(), encoded);
    }

    #[test]
    fn empty_properties_round_trip() {
        let encoded = encode_properties(&Properties::new()).unwrap();
        assert_eq!(encoded, r#"{"version":1,"properties":{}}"#);
        assert!(decode_properties(&encoded).unwrap().is_empty());
    }

    #[test]
    fn non_finite_floats_never_reach_storage() {
        // Properties itself refuses them...
        let mut properties = Properties::new();
        assert!(properties.insert(key("x"), f64::NAN).is_err());
        // ...and JSON cannot smuggle one back in.
        let overflow = r#"{"version":1,"properties":{"x":{"type":"float","value":1e400}}}"#;
        assert!(matches!(
            decode_properties(overflow),
            Err(GraphStoreError::CorruptData(_))
        ));
    }

    #[test]
    fn malformed_stored_data_is_corrupt_data() {
        let cases = [
            "not json",
            "[]",
            r#"{"properties":{}}"#,
            r#"{"version":2,"properties":{}}"#,
            r#"{"version":1,"properties":{},"extra":true}"#,
            r#"{"version":1,"properties":{"x":{"type":"decimal","value":"1.0"}}}"#,
            r#"{"version":1,"properties":{"x":{"type":"integer","value":"1"}}}"#,
            r#"{"version":1,"properties":{"x":{"type":"datetime","value":"yesterday"}}}"#,
            r#"{"version":1,"properties":{"bad key":{"type":"string","value":"v"}}}"#,
        ];
        for raw in cases {
            assert!(
                matches!(decode_properties(raw), Err(GraphStoreError::CorruptData(_))),
                "{raw} should be corrupt data"
            );
        }
    }

    #[test]
    fn vertex_ids_are_graph_scoped() {
        let (graph_id, entity_id) = (GraphId::new(), EntityId::new());
        let id = vertex_id(graph_id, entity_id);
        assert_eq!(parse_vertex_id(&json!(id)).unwrap(), (graph_id, entity_id));
        for bad in [
            json!(42),
            json!("no-separator"),
            json!("x:y"),
            json!(format!("{graph_id}:")),
        ] {
            assert!(matches!(
                parse_vertex_id(&bad),
                Err(GraphStoreError::CorruptData(_))
            ));
        }
    }

    fn sample_entity() -> Entity {
        let mut properties = Properties::new();
        properties.insert(key("name"), "Customer A").unwrap();
        Entity::new(
            EntityId::new(),
            GraphId::new(),
            TypeName::new("Customer").unwrap(),
            properties,
        )
    }

    fn as_vertex(new: NewVertex) -> Vertex {
        Vertex {
            id: new.id.into(),
            label: new.label.to_owned(),
            properties: new.properties,
        }
    }

    #[test]
    fn entities_round_trip_through_vertices() {
        let entity = sample_entity();
        let vertex = as_vertex(new_vertex(&entity).unwrap());
        assert_eq!(entity_from_vertex(&vertex).unwrap(), entity);
    }

    #[test]
    fn inconsistent_vertices_are_corrupt_data() {
        let entity = sample_entity();

        let mut wrong_graph = as_vertex(new_vertex(&entity).unwrap());
        wrong_graph
            .properties
            .insert(PROP_GRAPH_ID.into(), GraphId::new().to_string().into());
        let mut wrong_label = as_vertex(new_vertex(&entity).unwrap());
        wrong_label.label = "person".into();
        let mut missing_type = as_vertex(new_vertex(&entity).unwrap());
        missing_type.properties.remove(PROP_TYPE);
        let mut bad_type = as_vertex(new_vertex(&entity).unwrap());
        bad_type
            .properties
            .insert(PROP_TYPE.into(), "IP Address".into());

        for vertex in [wrong_graph, wrong_label, missing_type, bad_type] {
            assert!(matches!(
                entity_from_vertex(&vertex),
                Err(GraphStoreError::CorruptData(_))
            ));
        }
    }

    fn sample_relationship() -> Relationship {
        let graph_id = GraphId::new();
        let mut properties = Properties::new();
        properties.insert(key("amount"), 82_430.5).unwrap();
        Relationship::new(
            RelationshipId::new(),
            graph_id,
            TypeName::new("TRANSFERRED_TO").unwrap(),
            EntityId::new(),
            EntityId::new(),
            properties,
        )
    }

    fn as_edge(new: NewEdge) -> Edge {
        Edge {
            label: new.label.to_owned(),
            out_v: new.out_v.into(),
            in_v: new.in_v.into(),
            properties: new.properties,
        }
    }

    #[test]
    fn relationships_round_trip_through_edges() {
        let relationship = sample_relationship();
        let edge = as_edge(new_edge(&relationship).unwrap());
        assert_eq!(relationship_from_edge(&edge).unwrap(), relationship);
    }

    #[test]
    fn cross_graph_edges_are_corrupt_data() {
        let relationship = sample_relationship();
        let mut edge = as_edge(new_edge(&relationship).unwrap());
        edge.in_v = vertex_id(GraphId::new(), relationship.target_id()).into();
        assert!(matches!(
            relationship_from_edge(&edge),
            Err(GraphStoreError::CorruptData(_))
        ));
    }
}
