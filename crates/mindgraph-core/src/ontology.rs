//! Ontology use cases: the manual (non-AI) way to define a graph's schema.
//!
//! The AI Graph Builder (M5) produces an [`OntologyDefinition`] as a draft;
//! after review it is submitted through this same service. There is no
//! separate AI persistence path.
//!
//! The definition types serialise to the same JSON as the public
//! `POST /api/v1/graphs/{graph_id}/ontology` request, so an AI draft can be
//! reviewed and submitted unchanged.

use std::sync::Arc;

use mindgraph_domain::{
    EntityType, EntityTypeId, GraphId, Ontology, OntologyVersion, PropertiesSchema,
    PropertyDefinition, PropertyKey, PropertyType, RelationshipType, RelationshipTypeId, TypeName,
    WorkspaceId,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{ApplicationError, MetadataStore, MetadataStoreError};

/// One property of a type definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyInput {
    /// The property key, for example `customer_id`.
    pub name: String,
    /// The primitive type.
    #[serde(rename = "type")]
    pub property_type: PropertyType,
    /// Whether every entity or relationship of the type must have it.
    #[serde(default)]
    pub required: bool,
}

/// An entity type definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityTypeInput {
    /// The type name, for example `Customer`.
    pub name: String,
    /// A human-readable name; defaults to `name`.
    #[serde(default)]
    pub display_name: Option<String>,
    /// An optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// The properties.
    #[serde(default)]
    pub properties: Vec<PropertyInput>,
}

/// A relationship type definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipTypeInput {
    /// The type name, for example `OWNS`.
    pub name: String,
    /// The source entity type name.
    pub source_entity_type: String,
    /// The target entity type name.
    pub target_entity_type: String,
    /// The properties.
    #[serde(default)]
    pub properties: Vec<PropertyInput>,
}

/// The complete set of types of a new ontology version. It replaces the
/// previous version's types; it is not a patch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OntologyDefinition {
    /// The entity types.
    pub entity_types: Vec<EntityTypeInput>,
    /// The relationship types.
    #[serde(default)]
    pub relationship_types: Vec<RelationshipTypeInput>,
}

fn properties_of(schema: &PropertiesSchema) -> Vec<PropertyInput> {
    schema
        .definitions()
        .iter()
        .map(|definition| PropertyInput {
            name: definition.key.to_string(),
            property_type: definition.property_type,
            required: definition.required,
        })
        .collect()
}

impl OntologyDefinition {
    /// The types of an existing ontology version, without ids.
    #[must_use]
    pub fn from_ontology(ontology: &Ontology) -> Self {
        Self {
            entity_types: ontology
                .entity_types()
                .iter()
                .map(|entity_type| EntityTypeInput {
                    name: entity_type.name().to_string(),
                    display_name: Some(entity_type.display_name().to_owned()),
                    description: entity_type.description().map(str::to_owned),
                    properties: properties_of(entity_type.properties_schema()),
                })
                .collect(),
            relationship_types: ontology
                .relationship_types()
                .iter()
                .map(|relationship_type| RelationshipTypeInput {
                    name: relationship_type.name().to_string(),
                    source_entity_type: relationship_type.source_entity_type().to_string(),
                    target_entity_type: relationship_type.target_entity_type().to_string(),
                    properties: properties_of(relationship_type.properties_schema()),
                })
                .collect(),
        }
    }

    /// Builds the next version of `base` from this definition, validating
    /// every name and reference through the domain constructors. Nothing is
    /// stored.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::Validation`] for an invalid definition.
    pub fn build_next_version(&self, base: &Ontology) -> Result<Ontology, ApplicationError> {
        let graph_id = base.graph_id();
        let entity_types = self
            .entity_types
            .iter()
            .map(|input| entity_type(graph_id, input))
            .collect::<Result<_, _>>()?;
        let relationship_types = self
            .relationship_types
            .iter()
            .map(|input| relationship_type(graph_id, input))
            .collect::<Result<_, _>>()?;
        Ok(base.next_version(entity_types, relationship_types)?)
    }
}

/// Creates ontology versions.
#[derive(Clone)]
pub struct OntologyService {
    metadata: Arc<dyn MetadataStore>,
    workspace_id: WorkspaceId,
}

impl std::fmt::Debug for OntologyService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OntologyService")
            .field("workspace_id", &self.workspace_id)
            .finish_non_exhaustive()
    }
}

fn schema(graph_properties: &[PropertyInput]) -> Result<PropertiesSchema, ApplicationError> {
    let definitions = graph_properties
        .iter()
        .map(|property| {
            let key = PropertyKey::new(property.name.trim())?;
            Ok(if property.required {
                PropertyDefinition::required(key, property.property_type)
            } else {
                PropertyDefinition::optional(key, property.property_type)
            })
        })
        .collect::<Result<Vec<_>, ApplicationError>>()?;
    Ok(PropertiesSchema::new(definitions)?)
}

fn entity_type(graph_id: GraphId, input: &EntityTypeInput) -> Result<EntityType, ApplicationError> {
    let mut entity_type = EntityType::new(
        EntityTypeId::new(),
        graph_id,
        TypeName::new(input.name.trim())?,
        schema(&input.properties)?,
    );
    if let Some(display_name) = &input.display_name {
        entity_type = entity_type.with_display_name(display_name)?;
    }
    if let Some(description) = &input.description {
        entity_type = entity_type.with_description(description)?;
    }
    Ok(entity_type)
}

fn relationship_type(
    graph_id: GraphId,
    input: &RelationshipTypeInput,
) -> Result<RelationshipType, ApplicationError> {
    Ok(RelationshipType::new(
        RelationshipTypeId::new(),
        graph_id,
        TypeName::new(input.name.trim())?,
        TypeName::new(input.source_entity_type.trim())?,
        TypeName::new(input.target_entity_type.trim())?,
        schema(&input.properties)?,
    ))
}

impl OntologyService {
    /// A service for the graphs of `workspace_id`.
    #[must_use]
    pub fn new(metadata: Arc<dyn MetadataStore>, workspace_id: WorkspaceId) -> Self {
        Self {
            metadata,
            workspace_id,
        }
    }

    /// Builds the next ontology version from `definition` and makes it the
    /// active one, atomically. Type ids are generated here.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::GraphNotFound`], [`ApplicationError::Validation`]
    /// for an invalid definition (bad names, duplicates, unknown endpoint
    /// types), or [`ApplicationError::Conflict`] when another version was
    /// created concurrently.
    pub async fn create_version(
        &self,
        graph_id: GraphId,
        definition: &OntologyDefinition,
    ) -> Result<Ontology, ApplicationError> {
        self.create_version_expecting(graph_id, definition, None)
            .await
    }

    /// Like [`Self::create_version`], but only if the active version is
    /// still `expected_version`. The definition is the complete next
    /// ontology, so a client that read version N must not replace a version
    /// N+1 created in the meantime. The check is atomic: the new version is
    /// numbered `expected + 1`, and that number is unique per graph.
    ///
    /// # Errors
    ///
    /// As [`Self::create_version`], and [`ApplicationError::Conflict`] when
    /// the active version is not `expected_version`.
    pub async fn create_version_expecting(
        &self,
        graph_id: GraphId,
        definition: &OntologyDefinition,
        expected_version: Option<OntologyVersion>,
    ) -> Result<Ontology, ApplicationError> {
        self.metadata
            .get_graph(graph_id)
            .await?
            .filter(|graph| graph.workspace_id() == self.workspace_id)
            .ok_or(ApplicationError::GraphNotFound(graph_id))?;
        let active = self
            .metadata
            .get_active_ontology(graph_id)
            .await?
            .ok_or_else(|| {
                ApplicationError::internal(MetadataStoreError::CorruptData(format!(
                    "graph {graph_id} has no active ontology"
                )))
            })?;
        if let Some(expected) = expected_version
            && expected != active.version()
        {
            return Err(ApplicationError::Conflict(format!(
                "the active ontology is version {}, not {expected}; read it again and retry",
                active.version()
            )));
        }
        let ontology = definition.build_next_version(&active)?;
        self.metadata
            .create_and_activate_ontology(&ontology)
            .await
            .map_err(|err| match err {
                MetadataStoreError::Conflict(_) => ApplicationError::Conflict(
                    "the ontology changed concurrently; read it again and retry".to_owned(),
                ),
                other => other.into(),
            })?;
        info!(
            operation = "create_ontology_version",
            graph_id = %graph_id,
            version = %ontology.version(),
            entity_types = ontology.entity_types().len(),
            relationship_types = ontology.relationship_types().len(),
            "ontology version created and activated"
        );
        Ok(ontology)
    }
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::OntologyVersion;

    use super::*;
    use crate::{CreateGraphProject, GraphProjectService, MemoryMetadataStore};

    pub(crate) fn property(
        name: &str,
        property_type: PropertyType,
        required: bool,
    ) -> PropertyInput {
        PropertyInput {
            name: name.into(),
            property_type,
            required,
        }
    }

    async fn setup() -> (OntologyService, GraphProjectService, GraphId) {
        let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
        let graphs = GraphProjectService::bootstrap(metadata.clone())
            .await
            .unwrap();
        let graph = graphs
            .create_graph(CreateGraphProject {
                name: "Fraud".into(),
                description: None,
            })
            .await
            .unwrap();
        (
            OntologyService::new(metadata, graphs.workspace_id()),
            graphs,
            graph.id(),
        )
    }

    fn fraud() -> OntologyDefinition {
        OntologyDefinition {
            entity_types: vec![
                EntityTypeInput {
                    name: "Customer".into(),
                    display_name: Some("Customer".into()),
                    description: None,
                    properties: vec![
                        property("customer_id", PropertyType::String, true),
                        property("name", PropertyType::String, false),
                    ],
                },
                EntityTypeInput {
                    name: "Account".into(),
                    display_name: None,
                    description: Some("A bank account".into()),
                    properties: vec![property("account_id", PropertyType::String, true)],
                },
            ],
            relationship_types: vec![RelationshipTypeInput {
                name: "OWNS".into(),
                source_entity_type: "Customer".into(),
                target_entity_type: "Account".into(),
                properties: vec![],
            }],
        }
    }

    #[tokio::test]
    async fn creates_and_activates_the_next_version() {
        let (service, graphs, graph_id) = setup().await;
        let ontology = service.create_version(graph_id, &fraud()).await.unwrap();
        assert_eq!(ontology.version(), OntologyVersion::new(2).unwrap());
        assert_eq!(
            graphs.get_active_ontology(graph_id).await.unwrap(),
            ontology
        );
        let customer = ontology.entity_type("Customer").unwrap();
        assert!(
            customer
                .properties_schema()
                .get("customer_id")
                .unwrap()
                .required
        );
        assert_eq!(
            ontology.entity_type("Account").unwrap().description(),
            Some("A bank account")
        );

        let third = service
            .create_version(graph_id, &OntologyDefinition::default())
            .await
            .unwrap();
        assert_eq!(third.version(), OntologyVersion::new(3).unwrap());
        assert!(
            graphs
                .get_active_ontology(graph_id)
                .await
                .unwrap()
                .entity_types()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn an_expected_version_rejects_stale_definitions() {
        let (service, graphs, graph_id) = setup().await;
        let v2 = service
            .create_version_expecting(graph_id, &fraud(), Some(OntologyVersion::INITIAL))
            .await
            .unwrap();
        assert_eq!(v2.version(), OntologyVersion::new(2).unwrap());

        // A client that still holds version 1 would drop what version 2 added.
        let err = service
            .create_version_expecting(
                graph_id,
                &OntologyDefinition::default(),
                Some(OntologyVersion::INITIAL),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ApplicationError::Conflict(_)), "{err}");
        assert_eq!(graphs.get_active_ontology(graph_id).await.unwrap(), v2);

        let v3 = service
            .create_version_expecting(graph_id, &fraud(), Some(v2.version()))
            .await
            .unwrap();
        assert_eq!(v3.version(), OntologyVersion::new(3).unwrap());
    }

    #[tokio::test]
    async fn invalid_definitions_are_validation_errors_and_store_nothing() {
        let (service, graphs, graph_id) = setup().await;
        let mut duplicate = fraud();
        duplicate
            .entity_types
            .push(duplicate.entity_types[0].clone());
        let mut unknown_endpoint = fraud();
        unknown_endpoint.relationship_types[0].target_entity_type = "Merchant".into();
        let mut bad_name = fraud();
        bad_name.entity_types[0].name = "has space".into();
        let mut duplicate_property = fraud();
        duplicate_property.entity_types[1].properties.push(property(
            "account_id",
            PropertyType::Integer,
            false,
        ));
        for definition in [duplicate, unknown_endpoint, bad_name, duplicate_property] {
            let err = service
                .create_version(graph_id, &definition)
                .await
                .unwrap_err();
            assert!(matches!(err, ApplicationError::Validation(_)), "{err}");
        }
        let active = graphs.get_active_ontology(graph_id).await.unwrap();
        assert_eq!(active.version(), OntologyVersion::INITIAL);
    }

    #[tokio::test]
    async fn missing_graphs_are_not_found() {
        let (service, _, _) = setup().await;
        let err = service
            .create_version(GraphId::new(), &fraud())
            .await
            .unwrap_err();
        assert!(matches!(err, ApplicationError::GraphNotFound(_)));
    }
}
