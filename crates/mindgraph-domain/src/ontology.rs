//! Ontologies and ontology versions (ADR-003, ADR-005).

use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::{
    DomainError, Entity, EntityId, EntityType, GraphId, Relationship, RelationshipType, TypeName,
};

/// The version number of an ontology.
///
/// Versions start at 1 ([`Self::INITIAL`]) and increase by one with every
/// revision ([`Self::next`]). Each graph records which version is active
/// (ADR-005). Displayed as `v1`, `v2`, …, and serialised as a plain integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct OntologyVersion(NonZeroU32);

impl OntologyVersion {
    /// The first version of every ontology, `v1`.
    pub const INITIAL: Self = Self(NonZeroU32::MIN);

    /// Creates a version from its number.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidOntologyVersion`] for `0`.
    pub fn new(value: u32) -> Result<Self, DomainError> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or_else(|| DomainError::InvalidOntologyVersion("versions start at 1".to_owned()))
    }

    /// Returns the version number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }

    /// Returns the version that follows this one.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidOntologyVersion`] if the version number
    /// would overflow.
    pub fn next(self) -> Result<Self, DomainError> {
        self.0.checked_add(1).map(Self).ok_or_else(|| {
            DomainError::InvalidOntologyVersion(format!("{self} is the last representable version"))
        })
    }
}

impl fmt::Display for OntologyVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

impl TryFrom<u32> for OntologyVersion {
    type Error = DomainError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<OntologyVersion> for u32 {
    fn from(version: OntologyVersion) -> Self {
        version.get()
    }
}

/// One immutable version of a graph's schema: its entity types and
/// relationship types.
///
/// An `Ontology` is always internally consistent:
///
/// - every type belongs to the ontology's graph
/// - entity type names are unique, and so are relationship type names
/// - every relationship type's source and target entity types exist
///
/// To change the schema, build a new version with [`Self::next_version`].
/// An ontology without any types is valid. It is the starting point of the
/// "Start Empty" flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ontology {
    graph_id: GraphId,
    version: OntologyVersion,
    entity_types: Vec<EntityType>,
    relationship_types: Vec<RelationshipType>,
}

impl Ontology {
    /// Creates an ontology version, validating its consistency.
    ///
    /// Types are checked in the order given, so the first error reported is
    /// deterministic.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::GraphMismatch`],
    /// [`DomainError::DuplicateEntityType`],
    /// [`DomainError::DuplicateRelationshipType`] or
    /// [`DomainError::UnknownEntityType`].
    pub fn new(
        graph_id: GraphId,
        version: OntologyVersion,
        entity_types: Vec<EntityType>,
        relationship_types: Vec<RelationshipType>,
    ) -> Result<Self, DomainError> {
        let mut entity_names = BTreeSet::new();
        for entity_type in &entity_types {
            ensure_graph(
                graph_id,
                entity_type.graph_id(),
                "entity type",
                entity_type.name(),
            )?;
            if !entity_names.insert(entity_type.name().as_str()) {
                return Err(DomainError::DuplicateEntityType(
                    entity_type.name().to_string(),
                ));
            }
        }

        let mut relationship_names = BTreeSet::new();
        for relationship_type in &relationship_types {
            let name = relationship_type.name();
            ensure_graph(
                graph_id,
                relationship_type.graph_id(),
                "relationship type",
                name,
            )?;
            if !relationship_names.insert(name.as_str()) {
                return Err(DomainError::DuplicateRelationshipType(name.to_string()));
            }
            for endpoint in [
                relationship_type.source_entity_type(),
                relationship_type.target_entity_type(),
            ] {
                if !entity_names.contains(endpoint.as_str()) {
                    return Err(DomainError::UnknownEntityType(endpoint.to_string()));
                }
            }
        }

        Ok(Self {
            graph_id,
            version,
            entity_types,
            relationship_types,
        })
    }

    /// Creates the next version of this ontology for the same graph, with a
    /// new set of types. The current version is left unchanged.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::new`], or
    /// [`DomainError::InvalidOntologyVersion`] if the version would overflow.
    pub fn next_version(
        &self,
        entity_types: Vec<EntityType>,
        relationship_types: Vec<RelationshipType>,
    ) -> Result<Self, DomainError> {
        Self::new(
            self.graph_id,
            self.version.next()?,
            entity_types,
            relationship_types,
        )
    }

    /// The graph this ontology describes.
    #[must_use]
    pub const fn graph_id(&self) -> GraphId {
        self.graph_id
    }

    /// The version of this ontology.
    #[must_use]
    pub const fn version(&self) -> OntologyVersion {
        self.version
    }

    /// The entity types, in the order they were defined.
    #[must_use]
    pub fn entity_types(&self) -> &[EntityType] {
        &self.entity_types
    }

    /// The relationship types, in the order they were defined.
    #[must_use]
    pub fn relationship_types(&self) -> &[RelationshipType] {
        &self.relationship_types
    }

    /// Looks up an entity type by name.
    #[must_use]
    pub fn entity_type(&self, name: &str) -> Option<&EntityType> {
        self.entity_types
            .iter()
            .find(|entity_type| entity_type.name().as_str() == name)
    }

    /// Looks up a relationship type by name.
    #[must_use]
    pub fn relationship_type(&self, name: &str) -> Option<&RelationshipType> {
        self.relationship_types
            .iter()
            .find(|relationship_type| relationship_type.name().as_str() == name)
    }

    /// Checks that `entity` belongs to this ontology's graph, has a known
    /// entity type, and that its properties match that type's schema.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::GraphMismatch`],
    /// [`DomainError::UnknownEntityType`] or a property error.
    pub fn validate_entity(&self, entity: &Entity) -> Result<(), DomainError> {
        ensure_graph(self.graph_id, entity.graph_id(), "entity", entity.id())?;
        let entity_type = self
            .entity_type(entity.entity_type().as_str())
            .ok_or_else(|| DomainError::UnknownEntityType(entity.entity_type().to_string()))?;
        entity_type
            .properties_schema()
            .validate(entity.properties())
    }

    /// Checks that `relationship` is valid in this ontology, given its
    /// `source` and `target` entities:
    ///
    /// - the relationship and both entities belong to this ontology's graph
    /// - the relationship type exists
    /// - `source` and `target` are the entities the relationship refers to
    /// - their entity types match the relationship type's endpoints
    /// - the relationship's properties match the relationship type's schema
    ///
    /// The endpoint entities' own properties are not re-validated. Use
    /// [`Self::validate_entity`] for that.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::GraphMismatch`],
    /// [`DomainError::UnknownRelationshipType`],
    /// [`DomainError::EndpointMismatch`],
    /// [`DomainError::EndpointTypeMismatch`] or a property error.
    pub fn validate_relationship(
        &self,
        relationship: &Relationship,
        source: &Entity,
        target: &Entity,
    ) -> Result<(), DomainError> {
        ensure_graph(
            self.graph_id,
            relationship.graph_id(),
            "relationship",
            relationship.id(),
        )?;
        let relationship_type = self
            .relationship_type(relationship.relationship_type().as_str())
            .ok_or_else(|| {
                DomainError::UnknownRelationshipType(relationship.relationship_type().to_string())
            })?;

        self.check_endpoint(
            relationship_type,
            "source",
            relationship.source_id(),
            source,
            relationship_type.source_entity_type(),
        )?;
        self.check_endpoint(
            relationship_type,
            "target",
            relationship.target_id(),
            target,
            relationship_type.target_entity_type(),
        )?;

        relationship_type
            .properties_schema()
            .validate(relationship.properties())
    }

    fn check_endpoint(
        &self,
        relationship_type: &RelationshipType,
        endpoint: &'static str,
        expected_id: EntityId,
        entity: &Entity,
        required_type: &TypeName,
    ) -> Result<(), DomainError> {
        if entity.id() != expected_id {
            return Err(DomainError::EndpointMismatch {
                endpoint,
                expected: expected_id,
                found: entity.id(),
            });
        }
        ensure_graph(self.graph_id, entity.graph_id(), "entity", entity.id())?;
        if entity.entity_type() != required_type {
            return Err(DomainError::EndpointTypeMismatch {
                relationship_type: relationship_type.name().to_string(),
                endpoint,
                expected: required_type.to_string(),
                found: entity.entity_type().to_string(),
            });
        }
        Ok(())
    }
}

/// Fails with [`DomainError::GraphMismatch`] unless `found == expected`. The
/// name is only formatted on failure.
fn ensure_graph(
    expected: GraphId,
    found: GraphId,
    kind: &'static str,
    name: impl fmt::Display,
) -> Result<(), DomainError> {
    if found == expected {
        Ok(())
    } else {
        Err(DomainError::GraphMismatch {
            kind,
            name: name.to_string(),
            expected,
            found,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        EntityTypeId, Properties, PropertiesSchema, PropertyDefinition, PropertyKey, PropertyType,
        RelationshipId, RelationshipTypeId,
    };

    fn name(value: &str) -> TypeName {
        TypeName::new(value).unwrap()
    }

    fn key(value: &str) -> PropertyKey {
        PropertyKey::new(value).unwrap()
    }

    fn entity_type(
        graph_id: GraphId,
        type_name: &str,
        props: &[(&str, PropertyType)],
    ) -> EntityType {
        let schema = PropertiesSchema::new(
            props
                .iter()
                .map(|(k, t)| PropertyDefinition::required(key(k), *t))
                .collect(),
        )
        .unwrap();
        EntityType::new(EntityTypeId::new(), graph_id, name(type_name), schema)
    }

    fn relationship_type(
        graph_id: GraphId,
        type_name: &str,
        source: &str,
        target: &str,
        props: &[(&str, PropertyType)],
    ) -> RelationshipType {
        let schema = PropertiesSchema::new(
            props
                .iter()
                .map(|(k, t)| PropertyDefinition::required(key(k), *t))
                .collect(),
        )
        .unwrap();
        RelationshipType::new(
            RelationshipTypeId::new(),
            graph_id,
            name(type_name),
            name(source),
            name(target),
            schema,
        )
    }

    /// Customer ──OWNS──> Account, Customer ──USES──> Device,
    /// Account ──TRANSFERRED_TO {amount}──> Account.
    fn fraud_ontology(graph_id: GraphId) -> Ontology {
        Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![
                entity_type(graph_id, "Customer", &[("name", PropertyType::String)]),
                entity_type(
                    graph_id,
                    "Account",
                    &[("account_number", PropertyType::String)],
                ),
                entity_type(graph_id, "Device", &[]),
            ],
            vec![
                relationship_type(graph_id, "OWNS", "Customer", "Account", &[]),
                relationship_type(graph_id, "USES", "Customer", "Device", &[]),
                relationship_type(
                    graph_id,
                    "TRANSFERRED_TO",
                    "Account",
                    "Account",
                    &[("amount", PropertyType::Float)],
                ),
            ],
        )
        .unwrap()
    }

    fn entity(
        graph_id: GraphId,
        type_name: &str,
        props: serde_json::Value,
        ontology: &Ontology,
    ) -> Entity {
        let schema = ontology.entity_type(type_name).unwrap().properties_schema();
        let properties = Properties::from_json(&props, schema).unwrap();
        Entity::new(EntityId::new(), graph_id, name(type_name), properties)
    }

    fn relationship(
        graph_id: GraphId,
        type_name: &str,
        source: &Entity,
        target: &Entity,
    ) -> Relationship {
        Relationship::new(
            RelationshipId::new(),
            graph_id,
            name(type_name),
            source.id(),
            target.id(),
            Properties::new(),
        )
    }

    // --- OntologyVersion ----------------------------------------------------

    #[test]
    fn version_zero_is_invalid() {
        assert_eq!(
            OntologyVersion::new(0),
            Err(DomainError::InvalidOntologyVersion(
                "versions start at 1".to_owned()
            ))
        );
    }

    #[test]
    fn initial_version_is_one() {
        assert_eq!(OntologyVersion::INITIAL.get(), 1);
        assert_eq!(OntologyVersion::new(1).unwrap(), OntologyVersion::INITIAL);
        assert_eq!(OntologyVersion::INITIAL.to_string(), "v1");
    }

    #[test]
    fn next_increments_by_one_and_orders_after() {
        let v1 = OntologyVersion::INITIAL;
        let v2 = v1.next().unwrap();
        let v3 = v2.next().unwrap();
        assert_eq!((v2.get(), v3.get()), (2, 3));
        assert!(v1 < v2 && v2 < v3);
    }

    #[test]
    fn next_fails_instead_of_overflowing() {
        let last = OntologyVersion::new(u32::MAX).unwrap();
        assert!(matches!(
            last.next(),
            Err(DomainError::InvalidOntologyVersion(_))
        ));
    }

    #[test]
    fn version_serialises_as_a_validated_integer() {
        let v3 = OntologyVersion::new(3).unwrap();
        assert_eq!(serde_json::to_string(&v3).unwrap(), "3");
        assert_eq!(serde_json::from_str::<OntologyVersion>("3").unwrap(), v3);
        assert!(serde_json::from_str::<OntologyVersion>("0").is_err());
        assert!(serde_json::from_str::<OntologyVersion>("-1").is_err());
    }

    #[test]
    fn next_version_keeps_the_graph_and_bumps_the_version() {
        let graph_id = GraphId::new();
        let v1 = fraud_ontology(graph_id);
        let v2 = v1
            .next_version(v1.entity_types().to_vec(), v1.relationship_types().to_vec())
            .unwrap();
        assert_eq!(v2.graph_id(), graph_id);
        assert_eq!(v2.version().get(), 2);
        assert_eq!(v1.version(), OntologyVersion::INITIAL, "v1 is unchanged");
    }

    #[test]
    fn next_version_is_validated_like_a_new_ontology() {
        let graph_id = GraphId::new();
        let v1 = fraud_ontology(graph_id);
        // Dropping `Account` while `OWNS` still points to it is invalid.
        let without_account: Vec<_> = v1
            .entity_types()
            .iter()
            .filter(|t| t.name().as_str() != "Account")
            .cloned()
            .collect();
        assert_eq!(
            v1.next_version(without_account, v1.relationship_types().to_vec()),
            Err(DomainError::UnknownEntityType("Account".to_owned()))
        );
    }

    // --- Ontology construction ------------------------------------------------

    #[test]
    fn valid_ontology_exposes_its_types_in_order() {
        let graph_id = GraphId::new();
        let ontology = fraud_ontology(graph_id);
        let entity_names: Vec<&str> = ontology
            .entity_types()
            .iter()
            .map(|t| t.name().as_str())
            .collect();
        assert_eq!(entity_names, ["Customer", "Account", "Device"]);
        assert_eq!(ontology.relationship_types().len(), 3);
        assert_eq!(ontology.graph_id(), graph_id);
        assert_eq!(ontology.version(), OntologyVersion::INITIAL);
    }

    #[test]
    fn types_can_be_looked_up_by_name() {
        let ontology = fraud_ontology(GraphId::new());
        let owns = ontology.relationship_type("OWNS").unwrap();
        assert_eq!(owns.source_entity_type().as_str(), "Customer");
        assert_eq!(owns.target_entity_type().as_str(), "Account");
        assert!(ontology.entity_type("Merchant").is_none());
        assert!(
            ontology.relationship_type("owns").is_none(),
            "lookups are case-sensitive"
        );
    }

    #[test]
    fn empty_ontology_is_valid() {
        let ontology =
            Ontology::new(GraphId::new(), OntologyVersion::INITIAL, vec![], vec![]).unwrap();
        assert!(ontology.entity_types().is_empty());
        assert!(ontology.relationship_types().is_empty());
    }

    #[test]
    fn duplicate_entity_type_names_are_rejected() {
        let graph_id = GraphId::new();
        let result = Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![
                entity_type(graph_id, "Customer", &[]),
                entity_type(graph_id, "Customer", &[]),
            ],
            vec![],
        );
        assert_eq!(
            result,
            Err(DomainError::DuplicateEntityType("Customer".to_owned()))
        );
    }

    #[test]
    fn duplicate_relationship_type_names_are_rejected() {
        let graph_id = GraphId::new();
        let result = Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![
                entity_type(graph_id, "Customer", &[]),
                entity_type(graph_id, "Account", &[]),
            ],
            vec![
                relationship_type(graph_id, "OWNS", "Customer", "Account", &[]),
                relationship_type(graph_id, "OWNS", "Account", "Customer", &[]),
            ],
        );
        assert_eq!(
            result,
            Err(DomainError::DuplicateRelationshipType("OWNS".to_owned()))
        );
    }

    #[test]
    fn relationship_source_entity_type_must_exist() {
        let graph_id = GraphId::new();
        let result = Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![entity_type(graph_id, "Account", &[])],
            vec![relationship_type(
                graph_id,
                "OWNS",
                "Customer",
                "Account",
                &[],
            )],
        );
        assert_eq!(
            result,
            Err(DomainError::UnknownEntityType("Customer".to_owned()))
        );
    }

    #[test]
    fn relationship_target_entity_type_must_exist() {
        let graph_id = GraphId::new();
        let result = Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![entity_type(graph_id, "Customer", &[])],
            vec![relationship_type(
                graph_id,
                "OWNS",
                "Customer",
                "Account",
                &[],
            )],
        );
        assert_eq!(
            result,
            Err(DomainError::UnknownEntityType("Account".to_owned()))
        );
    }

    #[test]
    fn types_from_another_graph_are_rejected() {
        let graph_id = GraphId::new();
        let other = GraphId::new();

        let result = Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![entity_type(other, "Customer", &[])],
            vec![],
        );
        assert_eq!(
            result,
            Err(DomainError::GraphMismatch {
                kind: "entity type",
                name: "Customer".to_owned(),
                expected: graph_id,
                found: other,
            })
        );

        let result = Ontology::new(
            graph_id,
            OntologyVersion::INITIAL,
            vec![entity_type(graph_id, "Customer", &[])],
            vec![relationship_type(
                other,
                "KNOWS",
                "Customer",
                "Customer",
                &[],
            )],
        );
        assert!(matches!(
            result,
            Err(DomainError::GraphMismatch {
                kind: "relationship type",
                ..
            })
        ));
    }

    // --- Entity validation ----------------------------------------------------

    #[test]
    fn conforming_entities_are_valid() {
        let graph_id = GraphId::new();
        let ontology = fraud_ontology(graph_id);
        let customer = entity(graph_id, "Customer", json!({ "name": "Ikhwan" }), &ontology);
        assert_eq!(ontology.validate_entity(&customer), Ok(()));
    }

    #[test]
    fn entities_of_unknown_types_are_invalid() {
        let graph_id = GraphId::new();
        let ontology = fraud_ontology(graph_id);
        let merchant = Entity::new(
            EntityId::new(),
            graph_id,
            name("Merchant"),
            Properties::new(),
        );
        assert_eq!(
            ontology.validate_entity(&merchant),
            Err(DomainError::UnknownEntityType("Merchant".to_owned()))
        );
    }

    #[test]
    fn entities_must_satisfy_their_type_schema() {
        let graph_id = GraphId::new();
        let ontology = fraud_ontology(graph_id);
        let nameless = Entity::new(
            EntityId::new(),
            graph_id,
            name("Customer"),
            Properties::new(),
        );
        assert_eq!(
            ontology.validate_entity(&nameless),
            Err(DomainError::MissingRequiredProperty("name".to_owned()))
        );
    }

    #[test]
    fn entities_from_another_graph_are_invalid() {
        let graph_id = GraphId::new();
        let ontology = fraud_ontology(graph_id);
        let stranger = Entity::new(
            EntityId::new(),
            GraphId::new(),
            name("Device"),
            Properties::new(),
        );
        assert!(matches!(
            ontology.validate_entity(&stranger),
            Err(DomainError::GraphMismatch { kind: "entity", .. })
        ));
    }

    // --- Relationship validation ----------------------------------------------

    struct Fixture {
        graph_id: GraphId,
        ontology: Ontology,
        customer: Entity,
        account: Entity,
        device: Entity,
    }

    fn fixture() -> Fixture {
        let graph_id = GraphId::new();
        let ontology = fraud_ontology(graph_id);
        let customer = entity(graph_id, "Customer", json!({ "name": "Ikhwan" }), &ontology);
        let account = entity(
            graph_id,
            "Account",
            json!({ "account_number": "ACC-1" }),
            &ontology,
        );
        let device = entity(graph_id, "Device", json!({}), &ontology);
        Fixture {
            graph_id,
            ontology,
            customer,
            account,
            device,
        }
    }

    #[test]
    fn valid_relationships_pass() {
        let f = fixture();
        let owns = relationship(f.graph_id, "OWNS", &f.customer, &f.account);
        assert_eq!(
            f.ontology
                .validate_relationship(&owns, &f.customer, &f.account),
            Ok(())
        );

        let mut properties = Properties::new();
        properties.insert(key("amount"), 82_430.0).unwrap();
        let transfer = Relationship::new(
            RelationshipId::new(),
            f.graph_id,
            name("TRANSFERRED_TO"),
            f.account.id(),
            f.account.id(),
            properties,
        );
        assert_eq!(
            f.ontology
                .validate_relationship(&transfer, &f.account, &f.account),
            Ok(()),
            "self-loops are allowed"
        );
    }

    #[test]
    fn unknown_relationship_types_are_invalid() {
        let f = fixture();
        let knows = relationship(f.graph_id, "KNOWS", &f.customer, &f.customer);
        assert_eq!(
            f.ontology
                .validate_relationship(&knows, &f.customer, &f.customer),
            Err(DomainError::UnknownRelationshipType("KNOWS".to_owned()))
        );
    }

    #[test]
    fn wrong_source_entity_type_is_invalid() {
        let f = fixture();
        // OWNS requires Customer -> Account; Device -> Account is wrong.
        let owns = relationship(f.graph_id, "OWNS", &f.device, &f.account);
        assert_eq!(
            f.ontology
                .validate_relationship(&owns, &f.device, &f.account),
            Err(DomainError::EndpointTypeMismatch {
                relationship_type: "OWNS".to_owned(),
                endpoint: "source",
                expected: "Customer".to_owned(),
                found: "Device".to_owned(),
            })
        );
    }

    #[test]
    fn wrong_target_entity_type_is_invalid() {
        let f = fixture();
        let owns = relationship(f.graph_id, "OWNS", &f.customer, &f.device);
        assert_eq!(
            f.ontology
                .validate_relationship(&owns, &f.customer, &f.device),
            Err(DomainError::EndpointTypeMismatch {
                relationship_type: "OWNS".to_owned(),
                endpoint: "target",
                expected: "Account".to_owned(),
                found: "Device".to_owned(),
            })
        );
    }

    #[test]
    fn endpoints_must_be_the_referenced_entities() {
        let f = fixture();
        let owns = relationship(f.graph_id, "OWNS", &f.customer, &f.account);
        let other_customer = entity(f.graph_id, "Customer", json!({ "name": "B" }), &f.ontology);
        assert_eq!(
            f.ontology
                .validate_relationship(&owns, &other_customer, &f.account),
            Err(DomainError::EndpointMismatch {
                endpoint: "source",
                expected: f.customer.id(),
                found: other_customer.id(),
            })
        );
    }

    #[test]
    fn relationships_from_another_graph_are_invalid() {
        let f = fixture();
        let foreign = relationship(GraphId::new(), "OWNS", &f.customer, &f.account);
        assert!(matches!(
            f.ontology
                .validate_relationship(&foreign, &f.customer, &f.account),
            Err(DomainError::GraphMismatch {
                kind: "relationship",
                ..
            })
        ));
    }

    #[test]
    fn endpoints_from_another_graph_are_invalid() {
        let f = fixture();
        let foreign_account = Entity::new(
            EntityId::new(),
            GraphId::new(),
            name("Account"),
            Properties::new(),
        );
        let owns = relationship(f.graph_id, "OWNS", &f.customer, &foreign_account);
        assert!(matches!(
            f.ontology
                .validate_relationship(&owns, &f.customer, &foreign_account),
            Err(DomainError::GraphMismatch { kind: "entity", .. })
        ));
    }

    #[test]
    fn relationship_properties_must_satisfy_the_schema() {
        let f = fixture();
        let transfer = relationship(f.graph_id, "TRANSFERRED_TO", &f.account, &f.account);
        assert_eq!(
            f.ontology
                .validate_relationship(&transfer, &f.account, &f.account),
            Err(DomainError::MissingRequiredProperty("amount".to_owned()))
        );
    }

    #[test]
    fn relationship_endpoints_cannot_be_empty_ids() {
        // An empty or nil endpoint id cannot even be constructed, so a
        // `Relationship` can never point at "nothing".
        assert!("".parse::<EntityId>().is_err());
        assert!(EntityId::from_uuid(uuid::Uuid::nil()).is_err());
    }
}
