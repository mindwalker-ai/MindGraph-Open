//! End-to-end exercise of the public domain API using the fraud scenario
//! from the PRD: workspace → graph → ontology → entities → relationships →
//! ontology revision. Runs without any database, network or AI service.

// `allow-unwrap-in-tests` (clippy.toml) covers `#[test]` functions but not the
// fixture helpers of an integration-test crate. Panicking on unexpected
// errors is the intended failure mode for these fixtures.
#![allow(clippy::unwrap_used)]

use mindgraph_domain::{
    DomainError, Entity, EntityId, EntityType, EntityTypeId, GraphId, GraphProject, GraphStatus,
    Ontology, OntologyVersion, Properties, PropertiesSchema, PropertyDefinition, PropertyKey,
    PropertyType, PropertyValue, Relationship, RelationshipId, RelationshipType,
    RelationshipTypeId, TypeName, Workspace, WorkspaceId,
};
use serde_json::json;

fn name(value: &str) -> TypeName {
    TypeName::new(value).unwrap()
}

fn key(value: &str) -> PropertyKey {
    PropertyKey::new(value).unwrap()
}

fn new_graph(graph_name: &str) -> GraphProject {
    let workspace = Workspace::new(WorkspaceId::new(), "Local").unwrap();
    GraphProject::new(GraphId::new(), workspace.id(), graph_name, None).unwrap()
}

fn customer_type(graph: &GraphProject, extra: Vec<PropertyDefinition>) -> EntityType {
    let mut definitions = vec![
        PropertyDefinition::required(key("customer_id"), PropertyType::String),
        PropertyDefinition::required(key("name"), PropertyType::String),
        PropertyDefinition::optional(key("email"), PropertyType::String),
    ];
    definitions.extend(extra);
    EntityType::new(
        EntityTypeId::new(),
        graph.id(),
        name("Customer"),
        PropertiesSchema::new(definitions).unwrap(),
    )
}

fn device_type(graph: &GraphProject) -> EntityType {
    let schema = PropertiesSchema::new(vec![PropertyDefinition::required(
        key("device_id"),
        PropertyType::String,
    )])
    .unwrap();
    EntityType::new(EntityTypeId::new(), graph.id(), name("Device"), schema)
}

fn uses_type(graph: &GraphProject) -> RelationshipType {
    let schema = PropertiesSchema::new(vec![PropertyDefinition::optional(
        key("first_seen"),
        PropertyType::DateTime,
    )])
    .unwrap();
    RelationshipType::new(
        RelationshipTypeId::new(),
        graph.id(),
        name("USES"),
        name("Customer"),
        name("Device"),
        schema,
    )
}

fn entity_from_json(ontology: &Ontology, type_name: &str, json: serde_json::Value) -> Entity {
    let schema = ontology.entity_type(type_name).unwrap().properties_schema();
    let properties = Properties::from_json(&json, schema).unwrap();
    Entity::new(
        EntityId::new(),
        ontology.graph_id(),
        name(type_name),
        properties,
    )
}

#[test]
fn fraud_ring_can_be_modelled_and_validated() {
    let graph = new_graph("Fraud Investigation");
    assert_eq!(graph.status(), GraphStatus::Draft);

    let ontology = Ontology::new(
        graph.id(),
        OntologyVersion::INITIAL,
        vec![customer_type(&graph, vec![]), device_type(&graph)],
        vec![uses_type(&graph)],
    )
    .unwrap();

    // Two "unrelated" customers that share DEVICE-921.
    let customer_a = entity_from_json(
        &ontology,
        "Customer",
        json!({ "customer_id": "CUST-10291", "name": "Customer A", "email": "a@example.com" }),
    );
    let customer_b = entity_from_json(
        &ontology,
        "Customer",
        json!({ "customer_id": "CUST-20417", "name": "Customer B" }),
    );
    let device = entity_from_json(&ontology, "Device", json!({ "device_id": "DEVICE-921" }));

    for entity in [&customer_a, &customer_b, &device] {
        ontology.validate_entity(entity).unwrap();
    }

    // Source identifiers stay properties; MindGraph IDs are UUIDv7.
    assert_eq!(
        customer_a.properties().get("customer_id"),
        Some(&PropertyValue::from("CUST-10291"))
    );
    assert_eq!(customer_a.id().as_uuid().get_version_num(), 7);

    let uses_schema = ontology
        .relationship_type("USES")
        .unwrap()
        .properties_schema();
    let uses_props = Properties::from_json(
        &json!({ "first_seen": "2026-10-30T22:15:00+07:00" }),
        uses_schema,
    )
    .unwrap();
    for customer in [&customer_a, &customer_b] {
        let uses = Relationship::new(
            RelationshipId::new(),
            graph.id(),
            name("USES"),
            customer.id(),
            device.id(),
            uses_props.clone(),
        );
        ontology
            .validate_relationship(&uses, customer, &device)
            .unwrap();
    }

    // The reverse direction (Device USES Customer) violates the ontology.
    let reversed = Relationship::new(
        RelationshipId::new(),
        graph.id(),
        name("USES"),
        device.id(),
        customer_a.id(),
        Properties::new(),
    );
    assert!(matches!(
        ontology.validate_relationship(&reversed, &device, &customer_a),
        Err(DomainError::EndpointTypeMismatch {
            endpoint: "source",
            ..
        })
    ));
}

#[test]
fn entities_are_validated_against_a_specific_ontology_version() {
    let graph = new_graph("Fraud");
    let v1 = Ontology::new(
        graph.id(),
        OntologyVersion::INITIAL,
        vec![customer_type(&graph, vec![]), device_type(&graph)],
        vec![uses_type(&graph)],
    )
    .unwrap();
    let customer = entity_from_json(
        &v1,
        "Customer",
        json!({ "customer_id": "CUST-1", "name": "Ikhwan" }),
    );

    // v2 makes a new property mandatory.
    let risk_score = PropertyDefinition::required(key("risk_score"), PropertyType::Float);
    let v2 = v1
        .next_version(
            vec![customer_type(&graph, vec![risk_score]), device_type(&graph)],
            vec![uses_type(&graph)],
        )
        .unwrap();

    assert_eq!(v2.version().to_string(), "v2");
    assert!(v2.version() > v1.version());
    assert_eq!(v1.validate_entity(&customer), Ok(()));
    assert_eq!(
        v2.validate_entity(&customer),
        Err(DomainError::MissingRequiredProperty(
            "risk_score".to_owned()
        ))
    );
}
