//! AI Graph Builder use cases (M5).
//!
//! ```text
//! start_suggestion ─► preflight (graph, analysed data source, AI status)
//!                  ─► ai_suggest job (ontology version pinned, Idempotency-Key)
//! worker ─► GraphBuilderAi::suggest(profile + pinned ontology)
//!        ─► Rust validation (authoritative) ─► at most N repair attempts
//!        ─► job result: the validated suggestion
//! ```
//!
//! AI suggests, Rust validates, the user reviews, and M4 executes: the
//! reviewed ontology and mapping are submitted through `OntologyService`
//! and `IngestionService`. Nothing here writes ontologies, mappings or graph
//! data.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use mindgraph_domain::{DataSourceId, DataSourceStatus, GraphId, Ontology, PropertiesSchema};
use mindgraph_ingestion::{DatasetProfile, validate_mapping};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::executor::{JobExecutor, JobHandler};
use crate::ingestion::{metadata_failure, validate_idempotency_key};
use crate::{
    AiError, AiStatus, ApplicationError, GraphBuilderAi, GraphBuilderRequest,
    GraphBuilderSuggestion, IngestionJob, IngestionService, JobError, JobKind, JobResult,
    MetadataStore, OntologyContext, OntologyDefinition, RepairContext,
};

/// Largest number of entity types a suggestion may contain.
pub const MAX_SUGGESTED_ENTITY_TYPES: usize = 25;
/// Largest number of relationship types a suggestion may contain.
pub const MAX_SUGGESTED_RELATIONSHIP_TYPES: usize = 50;
/// Largest number of properties per suggested type.
pub const MAX_SUGGESTED_PROPERTIES: usize = 100;
/// Largest number of warnings kept.
pub const MAX_SUGGESTION_WARNINGS: usize = 20;
/// Warnings and reasons are truncated to this many characters.
pub const MAX_SUGGESTION_TEXT_CHARS: usize = 300;
/// Largest accepted `max_repair_attempts`.
pub const MAX_REPAIR_ATTEMPTS: u32 = 2;
/// Sample rows of the profile sent to the AI (the profile keeps up to 20).
const AI_SAMPLE_ROWS: usize = 10;
/// Upper bound of the serialised profile sent to the AI.
const MAX_AI_PROFILE_BYTES: usize = 96 * 1024;
/// Validation errors kept in a failed job's message.
const REPORTED_ERRORS: usize = 5;

/// Tuning of the AI Graph Builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphBuilderConfig {
    /// Repair attempts after Rust rejects a suggestion (`0..=2`).
    pub max_repair_attempts: u32,
}

impl Default for GraphBuilderConfig {
    fn default() -> Self {
        Self {
            max_repair_attempts: 1,
        }
    }
}

/// The persisted result of an `ai_suggest` job: a suggestion that passed
/// Rust validation, and its provenance. Never the prompt, the raw provider
/// response or any reasoning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphBuilderResult {
    /// The prompt version, for example `graph_builder_v1`.
    pub prompt_version: String,
    /// The provider kind.
    pub provider: String,
    /// The model name.
    pub model: String,
    /// The pinned ontology version the suggestion builds on.
    pub ontology_version_base: u32,
    /// How many repair attempts were needed.
    pub repair_attempts: u32,
    /// The validated suggestion.
    pub suggestion: GraphBuilderSuggestion,
}

/// Requests AI Graph Builder suggestions for analysed data sources.
#[derive(Clone)]
pub struct GraphBuilderService {
    ai: Arc<dyn GraphBuilderAi>,
    ingestion: IngestionService,
    executor: JobExecutor,
}

impl std::fmt::Debug for GraphBuilderService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphBuilderService")
            .finish_non_exhaustive()
    }
}

impl GraphBuilderService {
    /// Registers the `ai_suggest` worker on the shared job executor of
    /// `ingestion` and returns the service.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for more than
    /// [`MAX_REPAIR_ATTEMPTS`] repair attempts.
    pub fn new(
        ai: Arc<dyn GraphBuilderAi>,
        ingestion: &IngestionService,
        config: GraphBuilderConfig,
    ) -> Result<Self, ApplicationError> {
        if config.max_repair_attempts > MAX_REPAIR_ATTEMPTS {
            return Err(ApplicationError::InvalidRequest(format!(
                "at most {MAX_REPAIR_ATTEMPTS} repair attempts are allowed"
            )));
        }
        let executor = ingestion.executor().clone();
        executor.register(
            JobKind::AiSuggest,
            Arc::new(SuggestionWorker {
                metadata: Arc::clone(ingestion.metadata()),
                ai: Arc::clone(&ai),
                config,
            }),
        );
        Ok(Self {
            ai,
            ingestion: ingestion.clone(),
            executor,
        })
    }

    /// Whether suggestions can be requested now.
    pub async fn status(&self) -> AiStatus {
        self.ai.status().await
    }

    /// Queues an AI suggestion job for an analysed data source, pinned to
    /// the graph's active ontology. Submitting the same `idempotency_key`
    /// again for the graph returns the original job without another
    /// provider call.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for an invalid key, not-found
    /// errors, [`ApplicationError::Conflict`] if the data source is not
    /// analysed or the key was used for another request,
    /// [`ApplicationError::AiNotConfigured`] /
    /// [`ApplicationError::AiUnavailable`], and
    /// [`ApplicationError::Unavailable`] when the job queue is full.
    pub async fn start_suggestion(
        &self,
        graph_id: GraphId,
        data_source_id: DataSourceId,
        idempotency_key: &str,
    ) -> Result<IngestionJob, ApplicationError> {
        validate_idempotency_key(idempotency_key)?;
        let record = self.ingestion.record(graph_id, data_source_id).await?;
        let metadata = self.ingestion.metadata();
        if let Some(existing) = metadata
            .find_job_by_idempotency_key(graph_id, idempotency_key)
            .await?
        {
            return replay(existing, data_source_id);
        }
        if record.data_source.status() != DataSourceStatus::Ready || record.profile.is_none() {
            return Err(ApplicationError::Conflict(
                "the data source must be analysed successfully before an AI suggestion".to_owned(),
            ));
        }
        match self.ai.status().await {
            AiStatus::Ready { .. } => {}
            AiStatus::NotConfigured => return Err(ApplicationError::AiNotConfigured),
            AiStatus::Unavailable => return Err(ApplicationError::AiUnavailable),
        }
        let ontology = metadata
            .get_active_ontology(graph_id)
            .await?
            .ok_or(ApplicationError::GraphNotFound(graph_id))?;
        let job = IngestionJob::graph_builder(
            graph_id,
            data_source_id,
            ontology.version(),
            idempotency_key.to_owned(),
            Utc::now().trunc_subsecs(6),
        );
        match self.executor.submit(&job).await {
            Ok(()) => {}
            // A concurrent request with the same key won the race.
            Err(ApplicationError::Conflict(_)) => {
                let existing = metadata
                    .find_job_by_idempotency_key(graph_id, idempotency_key)
                    .await?
                    .ok_or_else(|| {
                        ApplicationError::Conflict("the job could not be created".to_owned())
                    })?;
                return replay(existing, data_source_id);
            }
            Err(other) => return Err(other),
        }
        info!(
            job_id = %job.id,
            graph_id = %graph_id,
            data_source_id = %data_source_id,
            kind = "ai_suggest",
            ontology_version = %ontology.version(),
            "job queued"
        );
        Ok(job)
    }
}

/// The original job for a repeated idempotency key, if the request matches.
fn replay(
    existing: IngestionJob,
    data_source_id: DataSourceId,
) -> Result<IngestionJob, ApplicationError> {
    if existing.kind == JobKind::AiSuggest && existing.data_source_id == data_source_id {
        Ok(existing)
    } else {
        Err(ApplicationError::Conflict(
            "this Idempotency-Key was already used for a different request".to_owned(),
        ))
    }
}

/// The profile as sent to the AI: at most [`AI_SAMPLE_ROWS`] sample rows,
/// fewer if the serialised profile would exceed [`MAX_AI_PROFILE_BYTES`].
fn ai_profile(profile: &DatasetProfile) -> DatasetProfile {
    let mut profile = profile.clone();
    profile.sample_rows.truncate(AI_SAMPLE_ROWS);
    let size = |p: &DatasetProfile| serde_json::to_vec(p).map_or(usize::MAX, |bytes| bytes.len());
    while size(&profile) > MAX_AI_PROFILE_BYTES && !profile.sample_rows.is_empty() {
        profile.sample_rows.pop();
    }
    if size(&profile) > MAX_AI_PROFILE_BYTES {
        for field in &mut profile.fields {
            field.sample_values.truncate(1);
        }
    }
    profile
}

fn truncate(text: &str) -> String {
    text.trim()
        .chars()
        .take(MAX_SUGGESTION_TEXT_CHARS)
        .collect()
}

fn valid_confidence(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

/// Rust's authoritative check of an AI suggestion against the pinned base
/// ontology and the dataset profile. Returns the normalised suggestion
/// (bounded texts, `existing` flags set by Rust) or every problem found,
/// phrased for a repair prompt: no internal details.
///
/// # Errors
///
/// The validation errors.
pub fn validate_suggestion(
    suggestion: &GraphBuilderSuggestion,
    base: &Ontology,
    profile: &DatasetProfile,
) -> Result<GraphBuilderSuggestion, Vec<String>> {
    let mut errors = Vec::new();
    let ontology = &suggestion.ontology;
    if ontology.entity_types.len() > MAX_SUGGESTED_ENTITY_TYPES {
        errors.push(format!(
            "at most {MAX_SUGGESTED_ENTITY_TYPES} entity types are allowed"
        ));
    }
    if ontology.relationship_types.len() > MAX_SUGGESTED_RELATIONSHIP_TYPES {
        errors.push(format!(
            "at most {MAX_SUGGESTED_RELATIONSHIP_TYPES} relationship types are allowed"
        ));
    }
    let property_counts = ontology
        .entity_types
        .iter()
        .map(|t| (&t.name, t.properties.len()))
        .chain(
            ontology
                .relationship_types
                .iter()
                .map(|t| (&t.name, t.properties.len())),
        );
    for (name, count) in property_counts {
        if count > MAX_SUGGESTED_PROPERTIES {
            errors.push(format!(
                "type `{name}` has more than {MAX_SUGGESTED_PROPERTIES} properties"
            ));
        }
    }
    let mut confidences = std::iter::once(suggestion.confidence).chain(
        suggestion
            .annotations
            .entities
            .iter()
            .chain(&suggestion.annotations.relationships)
            .map(|annotation| annotation.confidence),
    );
    if !confidences.all(valid_confidence) {
        errors.push("every confidence must be a number between 0 and 1".to_owned());
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let candidate = match ontology.build_next_version(base) {
        Ok(candidate) => candidate,
        Err(err) => return Err(vec![format!("invalid ontology: {err}")]),
    };
    check_preserves_base(base, &candidate, &mut errors);
    if let Err(err) = validate_mapping(&suggestion.mapping, &candidate, &profile.field_names()) {
        errors.push(err.to_string());
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut normalised = suggestion.clone();
    normalised.warnings = suggestion
        .warnings
        .iter()
        .map(|warning| truncate(warning))
        .filter(|warning| !warning.is_empty())
        .take(MAX_SUGGESTION_WARNINGS)
        .collect();
    normalised
        .annotations
        .entities
        .retain(|annotation| candidate.entity_type(annotation.name.trim()).is_some());
    normalised.annotations.relationships.retain(|annotation| {
        candidate
            .relationship_type(annotation.name.trim())
            .is_some()
    });
    for annotation in &mut normalised.annotations.entities {
        annotation.existing = base.entity_type(annotation.name.trim()).is_some();
        annotation.reason = truncate(&annotation.reason);
    }
    for annotation in &mut normalised.annotations.relationships {
        annotation.existing = base.relationship_type(annotation.name.trim()).is_some();
        annotation.reason = truncate(&annotation.reason);
    }
    Ok(normalised)
}

fn check_properties(
    owner: &str,
    base: &PropertiesSchema,
    candidate: &PropertiesSchema,
    errors: &mut Vec<String>,
) {
    for existing in base.definitions() {
        match candidate.get(existing.key.as_str()) {
            None => errors.push(format!(
                "existing property `{}` of `{owner}` must be kept",
                existing.key
            )),
            Some(kept) if kept.property_type != existing.property_type => errors.push(format!(
                "existing property `{}` of `{owner}` must keep type {}",
                existing.key, existing.property_type
            )),
            Some(kept) if kept.required != existing.required => errors.push(format!(
                "existing property `{}` of `{owner}` must stay {}",
                existing.key,
                if existing.required {
                    "required"
                } else {
                    "optional"
                }
            )),
            Some(_) => {}
        }
    }
    for added in candidate.definitions() {
        if added.required && base.get(added.key.as_str()).is_none() {
            errors.push(format!(
                "new property `{}` of existing type `{owner}` must be optional",
                added.key
            ));
        }
    }
}

/// AI suggestions are non-destructive: every existing type, property type,
/// requiredness and relationship endpoint is kept; existing types may only
/// gain optional properties.
fn check_preserves_base(base: &Ontology, candidate: &Ontology, errors: &mut Vec<String>) {
    for existing in base.entity_types() {
        let name = existing.name().as_str();
        match candidate.entity_type(name) {
            None => errors.push(format!("existing entity type `{name}` must be kept")),
            Some(kept) => check_properties(
                name,
                existing.properties_schema(),
                kept.properties_schema(),
                errors,
            ),
        }
    }
    for existing in base.relationship_types() {
        let name = existing.name().as_str();
        match candidate.relationship_type(name) {
            None => errors.push(format!("existing relationship type `{name}` must be kept")),
            Some(kept)
                if kept.source_entity_type() != existing.source_entity_type()
                    || kept.target_entity_type() != existing.target_entity_type() =>
            {
                errors.push(format!(
                    "existing relationship type `{name}` must keep its endpoints `{}` -> `{}`",
                    existing.source_entity_type(),
                    existing.target_entity_type()
                ));
            }
            Some(kept) => check_properties(
                name,
                existing.properties_schema(),
                kept.properties_schema(),
                errors,
            ),
        }
    }
}

fn ai_failure(error: &AiError) -> JobError {
    JobError::new(error.code(), error.to_string())
}

fn validation_failure(errors: &[String]) -> JobError {
    let shown: Vec<&str> = errors
        .iter()
        .take(REPORTED_ERRORS)
        .map(String::as_str)
        .collect();
    let mut message = format!("the AI suggestion is invalid: {}", shown.join("; "));
    let more = errors.len().saturating_sub(REPORTED_ERRORS);
    if more > 0 {
        message.push_str(&format!(" (and {more} more)"));
    }
    JobError::new(AiError::ValidationFailed(String::new()).code(), message)
}

/// The worker of `ai_suggest` jobs.
struct SuggestionWorker {
    metadata: Arc<dyn MetadataStore>,
    ai: Arc<dyn GraphBuilderAi>,
    config: GraphBuilderConfig,
}

impl SuggestionWorker {
    async fn progress(&self, job: &IngestionJob, value: f64) {
        if let Err(err) = self.metadata.update_job_progress(job.id, value).await {
            warn!(job_id = %job.id, error = %err, "could not persist job progress");
        }
    }
}

#[async_trait]
impl JobHandler for SuggestionWorker {
    async fn run(&self, job: &IngestionJob) -> Result<Option<JobResult>, JobError> {
        let profile = self
            .metadata
            .get_data_source(job.graph_id, job.data_source_id)
            .await
            .map_err(|err| metadata_failure(&err))?
            .and_then(|record| record.profile)
            .ok_or_else(|| {
                JobError::new("DATA_SOURCE_NOT_READY", "the data source has no profile")
            })?;
        let version = job
            .ontology_version
            .ok_or_else(|| JobError::new("INTERNAL_ERROR", "the job has no pinned ontology"))?;
        // The pinned version, not whichever version is active now.
        let base = self
            .metadata
            .get_ontology(job.graph_id, version)
            .await
            .map_err(|err| metadata_failure(&err))?
            .ok_or_else(|| {
                JobError::new(
                    "ONTOLOGY_NOT_FOUND",
                    "the pinned ontology version is missing",
                )
            })?;
        self.progress(job, 0.1).await;

        let started = Instant::now();
        let mut request = GraphBuilderRequest {
            correlation_id: job.id.to_string(),
            dataset_profile: ai_profile(&profile),
            active_ontology: OntologyContext {
                version: version.get(),
                definition: OntologyDefinition::from_ontology(&base),
            },
            repair: None,
        };
        let mut repair_attempts = 0;
        loop {
            let response = self
                .ai
                .suggest(&request)
                .await
                .map_err(|err| ai_failure(&err))?;
            match validate_suggestion(&response.suggestion, &base, &profile) {
                Ok(suggestion) => {
                    info!(
                        job_id = %job.id,
                        graph_id = %job.graph_id,
                        data_source_id = %job.data_source_id,
                        provider = %response.metadata.provider,
                        model = %response.metadata.model,
                        prompt_version = %response.metadata.prompt_version,
                        field_count = profile.fields.len(),
                        record_count = profile.record_count,
                        repair_attempts,
                        duration_ms =
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        "AI suggestion validated"
                    );
                    return Ok(Some(JobResult::GraphBuilder(Box::new(
                        GraphBuilderResult {
                            prompt_version: response.metadata.prompt_version,
                            provider: response.metadata.provider,
                            model: response.metadata.model,
                            ontology_version_base: version.get(),
                            repair_attempts,
                            suggestion,
                        },
                    ))));
                }
                Err(errors) if repair_attempts < self.config.max_repair_attempts => {
                    repair_attempts += 1;
                    // Only the count: errors may name fields of the dataset.
                    warn!(
                        job_id = %job.id,
                        errors = errors.len(),
                        repair_attempt = repair_attempts,
                        "AI suggestion rejected by validation; requesting a repair"
                    );
                    self.progress(job, 0.5).await;
                    request.repair = Some(RepairContext {
                        previous: response.suggestion,
                        errors,
                    });
                }
                Err(errors) => {
                    warn!(
                        job_id = %job.id,
                        errors = errors.len(),
                        repair_attempts,
                        "AI suggestion rejected by validation"
                    );
                    return Err(validation_failure(&errors));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_profiles_are_bounded() {
        let mut profile = DatasetProfile {
            record_count: 30,
            fields: vec![],
            sample_rows: vec![],
        };
        for i in 0..20 {
            let mut row = serde_json::Map::new();
            row.insert(
                "v".into(),
                serde_json::Value::String("x".repeat(20_000) + &i.to_string()),
            );
            profile.sample_rows.push(row);
        }
        let sent = ai_profile(&profile);
        assert!(sent.sample_rows.len() <= AI_SAMPLE_ROWS);
        assert!(serde_json::to_vec(&sent).unwrap().len() <= MAX_AI_PROFILE_BYTES);
        assert_eq!(sent.record_count, 30);
    }

    #[test]
    fn validation_failures_are_bounded_summaries() {
        let errors: Vec<String> = (0..8).map(|i| format!("error {i}")).collect();
        let failure = validation_failure(&errors);
        assert_eq!(failure.code, "AI_VALIDATION_FAILED");
        assert!(failure.message.contains("error 4") && !failure.message.contains("error 5"));
        assert!(failure.message.ends_with("(and 3 more)"));
    }
}
