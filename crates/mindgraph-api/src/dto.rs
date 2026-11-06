//! Public HTTP request and response bodies.
//!
//! These are deliberately separate from the domain types, so the public API
//! stays stable when internal structs evolve.

use chrono::{DateTime, SecondsFormat, Utc};
use mindgraph_core::{
    AiStatus, DataSourceView, EntityTypeInput, GraphBuilderResult, IngestionJob, JobResult,
    OntologyDefinition, PropertyInput, RelationshipTypeInput,
};
use mindgraph_domain::{
    EntityType, GraphProject, Ontology, OntologyVersion, PropertiesSchema, PropertyType,
    RelationshipType,
};
use mindgraph_ingestion::{DatasetProfile, IngestionMapping, IngestionSummary};
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

pub(crate) fn timestamp(at: DateTime<Utc>) -> String {
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

/// One property of `POST /api/v1/graphs/{graph_id}/ontology`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PropertyRequest {
    /// Also accepted as `key`, the name used in responses.
    #[serde(alias = "key")]
    name: String,
    #[serde(rename = "type")]
    property_type: PropertyType,
    #[serde(default)]
    required: bool,
}

impl From<PropertyRequest> for PropertyInput {
    fn from(request: PropertyRequest) -> Self {
        Self {
            name: request.name,
            property_type: request.property_type,
            required: request.required,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntityTypeRequest {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    properties: Vec<PropertyRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RelationshipTypeRequest {
    name: String,
    source_entity_type: String,
    target_entity_type: String,
    #[serde(default)]
    properties: Vec<PropertyRequest>,
}

/// `POST /api/v1/graphs/{graph_id}/graph-builder/suggestions`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SuggestionRequest {
    /// The analysed data sources to model together, in order.
    pub(crate) data_source_ids: Vec<String>,
}

/// `POST /api/v1/graphs/{graph_id}/ontology`: the complete next version.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateOntologyRequest {
    /// Optional precondition: the active version the definition was based
    /// on. A different active version is a conflict.
    #[serde(default)]
    pub(crate) expected_version: Option<OntologyVersion>,
    entity_types: Vec<EntityTypeRequest>,
    #[serde(default)]
    relationship_types: Vec<RelationshipTypeRequest>,
}

impl From<CreateOntologyRequest> for OntologyDefinition {
    fn from(request: CreateOntologyRequest) -> Self {
        let properties =
            |list: Vec<PropertyRequest>| list.into_iter().map(PropertyInput::from).collect();
        Self {
            entity_types: request
                .entity_types
                .into_iter()
                .map(|entity_type| EntityTypeInput {
                    name: entity_type.name,
                    display_name: entity_type.display_name,
                    description: entity_type.description,
                    properties: properties(entity_type.properties),
                })
                .collect(),
            relationship_types: request
                .relationship_types
                .into_iter()
                .map(|relationship_type| RelationshipTypeInput {
                    name: relationship_type.name,
                    source_entity_type: relationship_type.source_entity_type,
                    target_entity_type: relationship_type.target_entity_type,
                    properties: properties(relationship_type.properties),
                })
                .collect(),
        }
    }
}

/// A data source, without any storage detail (no paths, no storage key).
#[derive(Debug, Serialize)]
pub(crate) struct DataSourceResponse {
    id: String,
    graph_id: String,
    kind: &'static str,
    filename: String,
    status: &'static str,
    size_bytes: u64,
    content_sha256: String,
    profile: Option<DatasetProfile>,
    created_at: String,
    updated_at: String,
}

impl From<DataSourceView> for DataSourceResponse {
    fn from(view: DataSourceView) -> Self {
        let source = view.data_source;
        Self {
            id: source.id().to_string(),
            graph_id: source.graph_id().to_string(),
            kind: source.kind().as_str(),
            filename: source.filename().to_owned(),
            status: source.status().as_str(),
            size_bytes: source.size_bytes(),
            content_sha256: source.content_sha256().to_owned(),
            profile: view.profile,
            created_at: timestamp(source.created_at()),
            updated_at: timestamp(source.updated_at()),
        }
    }
}

/// `POST .../data-sources/{id}/ingestions`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IngestionRequest {
    pub(crate) mapping: IngestionMapping,
}

#[derive(Debug, Serialize)]
struct JobErrorResponse {
    code: String,
    message: String,
}

/// What a completed job produced, by job kind: ingestion counters, or the
/// validated AI suggestion. Analysis jobs have none (`null`).
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum JobResultResponse {
    Ingestion(IngestionSummary),
    GraphBuilder(Box<GraphBuilderResult>),
}

impl From<&JobResult> for JobResultResponse {
    fn from(result: &JobResult) -> Self {
        match result {
            JobResult::Ingestion(summary) => Self::Ingestion(*summary),
            JobResult::GraphBuilder(result) => Self::GraphBuilder(result.clone()),
        }
    }
}

/// A job. The mapping and idempotency key are not echoed.
#[derive(Debug, Serialize)]
pub(crate) struct JobResponse {
    id: String,
    graph_id: String,
    data_source_id: String,
    data_source_ids: Vec<String>,
    kind: &'static str,
    status: &'static str,
    progress: f64,
    ontology_version: Option<u32>,
    error: Option<JobErrorResponse>,
    result: Option<JobResultResponse>,
    created_at: String,
    updated_at: String,
}

impl From<&IngestionJob> for JobResponse {
    fn from(job: &IngestionJob) -> Self {
        Self {
            id: job.id.to_string(),
            graph_id: job.graph_id.to_string(),
            data_source_id: job.data_source_id.to_string(),
            data_source_ids: job
                .data_source_ids
                .iter()
                .map(ToString::to_string)
                .collect(),
            kind: job.kind.as_str(),
            status: job.status.as_str(),
            progress: job.progress,
            ontology_version: job.ontology_version.map(|version| version.get()),
            error: job.error.as_ref().map(|error| JobErrorResponse {
                code: error.code.clone(),
                message: error.message.clone(),
            }),
            result: job.result.as_ref().map(JobResultResponse::from),
            created_at: timestamp(job.created_at),
            updated_at: timestamp(job.updated_at),
        }
    }
}

/// `202 Accepted` of `POST .../ingestions`: the job, plus the `job_id`
/// every 202 response carries (ADR-006).
#[derive(Debug, Serialize)]
pub(crate) struct IngestionAcceptedResponse {
    job_id: String,
    #[serde(flatten)]
    job: JobResponse,
}

impl From<&IngestionJob> for IngestionAcceptedResponse {
    fn from(job: &IngestionJob) -> Self {
        Self {
            job_id: job.id.to_string(),
            job: JobResponse::from(job),
        }
    }
}

/// `202 Accepted` of `POST .../analyze`.
#[derive(Debug, Serialize)]
pub(crate) struct JobAcceptedResponse {
    job_id: String,
    status: &'static str,
}

impl From<&IngestionJob> for JobAcceptedResponse {
    fn from(job: &IngestionJob) -> Self {
        Self {
            job_id: job.id.to_string(),
            status: job.status.as_str(),
        }
    }
}

/// `GET /api/v1/graph-builder/status`. Never contains URLs or credentials.
#[derive(Debug, Serialize)]
pub(crate) struct GraphBuilderStatusResponse {
    status: &'static str,
    configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

impl From<AiStatus> for GraphBuilderStatusResponse {
    fn from(status: AiStatus) -> Self {
        match status {
            AiStatus::Ready { provider, model } => Self {
                status: "ready",
                configured: true,
                provider: Some(provider),
                model: Some(model),
            },
            AiStatus::NotConfigured => Self {
                status: "not_configured",
                configured: false,
                provider: None,
                model: None,
            },
            AiStatus::Unavailable => Self {
                status: "unavailable",
                configured: true,
                provider: None,
                model: None,
            },
        }
    }
}
