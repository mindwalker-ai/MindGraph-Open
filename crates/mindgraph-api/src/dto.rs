//! Public HTTP request and response bodies.
//!
//! These are deliberately separate from the domain types, so the public API
//! stays stable when internal structs evolve.

use chrono::{DateTime, SecondsFormat, Utc};
use mindgraph_domain::{EntityType, GraphProject, Ontology, PropertiesSchema, RelationshipType};
use serde::{Deserialize, Serialize};

/// `POST /api/v1/graphs`
#[derive(Debug, Deserialize)]
pub(crate) struct CreateGraphRequest {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

/// `GET /api/v1/graphs?limit=`
#[derive(Debug, Deserialize)]
pub(crate) struct ListGraphsQuery {
    pub(crate) limit: Option<usize>,
}

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

#[derive(Debug, Serialize)]
pub(crate) struct GraphResponse {
    id: String,
    workspace_id: String,
    name: String,
    description: Option<String>,
    status: &'static str,
    created_at: String,
    updated_at: String,
}

impl From<&GraphProject> for GraphResponse {
    fn from(graph: &GraphProject) -> Self {
        Self {
            id: graph.id().to_string(),
            workspace_id: graph.workspace_id().to_string(),
            name: graph.name().to_owned(),
            description: graph.description().map(str::to_owned),
            status: graph.status().as_str(),
            created_at: timestamp(graph.created_at()),
            updated_at: timestamp(graph.updated_at()),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct GraphListResponse {
    data: Vec<GraphResponse>,
    count: usize,
}

impl From<&[GraphProject]> for GraphListResponse {
    fn from(graphs: &[GraphProject]) -> Self {
        Self {
            data: graphs.iter().map(GraphResponse::from).collect(),
            count: graphs.len(),
        }
    }
}

#[derive(Debug, Serialize)]
struct PropertyResponse {
    key: String,
    #[serde(rename = "type")]
    property_type: &'static str,
    required: bool,
}

fn properties(schema: &PropertiesSchema) -> Vec<PropertyResponse> {
    schema
        .definitions()
        .iter()
        .map(|definition| PropertyResponse {
            key: definition.key.to_string(),
            property_type: definition.property_type.as_str(),
            required: definition.required,
        })
        .collect()
}

#[derive(Debug, Serialize)]
struct EntityTypeResponse {
    id: String,
    name: String,
    display_name: String,
    description: Option<String>,
    properties: Vec<PropertyResponse>,
}

impl From<&EntityType> for EntityTypeResponse {
    fn from(entity_type: &EntityType) -> Self {
        Self {
            id: entity_type.id().to_string(),
            name: entity_type.name().to_string(),
            display_name: entity_type.display_name().to_owned(),
            description: entity_type.description().map(str::to_owned),
            properties: properties(entity_type.properties_schema()),
        }
    }
}

#[derive(Debug, Serialize)]
struct RelationshipTypeResponse {
    id: String,
    name: String,
    source_entity_type: String,
    target_entity_type: String,
    properties: Vec<PropertyResponse>,
}

impl From<&RelationshipType> for RelationshipTypeResponse {
    fn from(relationship_type: &RelationshipType) -> Self {
        Self {
            id: relationship_type.id().to_string(),
            name: relationship_type.name().to_string(),
            source_entity_type: relationship_type.source_entity_type().to_string(),
            target_entity_type: relationship_type.target_entity_type().to_string(),
            properties: properties(relationship_type.properties_schema()),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct OntologyResponse {
    graph_id: String,
    version: u32,
    entity_types: Vec<EntityTypeResponse>,
    relationship_types: Vec<RelationshipTypeResponse>,
}

impl From<&Ontology> for OntologyResponse {
    fn from(ontology: &Ontology) -> Self {
        Self {
            graph_id: ontology.graph_id().to_string(),
            version: ontology.version().get(),
            entity_types: ontology.entity_types().iter().map(Into::into).collect(),
            relationship_types: ontology
                .relationship_types()
                .iter()
                .map(Into::into)
                .collect(),
        }
    }
}
