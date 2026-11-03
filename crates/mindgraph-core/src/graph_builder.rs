//! AI Graph Builder use cases (M5).
//!
//! ```text
//! start_suggestion ─► preflight (graph, analysed data sources, AI status)
//!                  ─► ai_suggest job (ontology version pinned, Idempotency-Key)
//! worker ─► GraphBuilderAi::suggest(profiles of every dataset + pinned ontology)
//!        ─► Rust validation (authoritative) ─► at most N repair attempts
//!        ─► job result: the validated suggestion, one mapping per dataset in a
//!           safe ingestion order
//! ```
//!
//! AI suggests, Rust validates, the user reviews, and M4 executes: the
//! reviewed ontology and each dataset's mapping are submitted through
//! `OntologyService` and `IngestionService`. Nothing here writes ontologies,
//! mappings or graph data.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use mindgraph_domain::{DataSourceId, DataSourceStatus, GraphId, Ontology, PropertiesSchema};
use mindgraph_ingestion::{DatasetProfile, EntityMode, validate_mapping};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::executor::{JobExecutor, JobHandler};
use crate::ingestion::{metadata_failure, validate_idempotency_key};
use crate::{
    AiError, AiStatus, ApplicationError, DataSourceRecord, DatasetContext, DatasetMapping,
    GraphBuilderAi, GraphBuilderRequest, GraphBuilderSuggestion, IngestionJob, IngestionService,
    JobError, JobKind, JobResult, MAX_SUGGESTION_DATA_SOURCES, MetadataStore, OntologyContext,
    OntologyDefinition, RepairContext,
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
/// Default number of AI jobs running at the same time.
pub const DEFAULT_AI_JOB_CONCURRENCY: usize = 2;
/// Largest accepted `job_concurrency`.
pub const MAX_AI_JOB_CONCURRENCY: usize = 8;
/// AI jobs waiting for a worker; more are rejected as unavailable.
const AI_QUEUE_CAPACITY: usize = 64;
/// Repair errors sent back to the AI service, and the characters kept per
/// error: the bounds of its `RepairContext` schema.
const MAX_REPAIR_ERRORS: usize = 50;
const MAX_REPAIR_ERROR_CHARS: usize = 500;
/// Largest serialised job result. PostgreSQL bounds `result_json` to
/// 512 KiB of its own text form, which adds whitespace, so stay well below.
const MAX_RESULT_BYTES: usize = 384 * 1024;
/// Sample rows of the profile sent to the AI (the profile keeps up to 20).
const AI_SAMPLE_ROWS: usize = 10;
/// Upper bound of one serialised profile sent to the AI.
const MAX_AI_PROFILE_BYTES: usize = 96 * 1024;
/// Upper bound of all profiles of one request together (the AI service
/// accepts requests of at most 512 KiB, ontology included).
const MAX_AI_PROFILES_BYTES: usize = 320 * 1024;
/// Validation errors kept in a failed job's message.
const REPORTED_ERRORS: usize = 5;

/// Tuning of the AI Graph Builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphBuilderConfig {
    /// Repair attempts after Rust rejects a suggestion (`0..=2`).
    pub max_repair_attempts: u32,
    /// AI jobs running at the same time (`1..=8`). AI jobs have their own
    /// workers, so slow model calls never delay analysis or ingestion.
    pub job_concurrency: usize,
}

impl Default for GraphBuilderConfig {
    fn default() -> Self {
        Self {
            max_repair_attempts: 1,
            job_concurrency: DEFAULT_AI_JOB_CONCURRENCY,
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
    /// Starts the `ai_suggest` workers and returns the service. They have
    /// their own queue and concurrency limit, separate from analysis and
    /// ingestion jobs, but the same job store. Must be called inside a Tokio
    /// runtime.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for more than
    /// [`MAX_REPAIR_ATTEMPTS`] repair attempts or a job concurrency outside
    /// `1..=MAX_AI_JOB_CONCURRENCY`.
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
        if !(1..=MAX_AI_JOB_CONCURRENCY).contains(&config.job_concurrency) {
            return Err(ApplicationError::InvalidRequest(format!(
                "the AI job concurrency must be between 1 and {MAX_AI_JOB_CONCURRENCY}"
            )));
        }
        let executor = JobExecutor::start(
            Arc::clone(ingestion.metadata()),
            config.job_concurrency,
            AI_QUEUE_CAPACITY,
        );
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

    /// Queues one AI suggestion job that models the analysed data sources
    /// `data_source_ids` together (`1..=MAX_SUGGESTION_DATA_SOURCES`,
    /// distinct, in the client's order), pinned to the graph's active
    /// ontology. Submitting the same `idempotency_key` again for the graph
    /// returns the original job without another provider call.
    ///
    /// # Errors
    ///
    /// [`ApplicationError::InvalidRequest`] for an invalid key or data
    /// source list, not-found errors, [`ApplicationError::Conflict`] if a
    /// data source is not analysed or the key was used for another request,
    /// [`ApplicationError::AiNotConfigured`] /
    /// [`ApplicationError::AiUnavailable`], and
    /// [`ApplicationError::Unavailable`] when the job queue is full.
    pub async fn start_suggestion(
        &self,
        graph_id: GraphId,
        data_source_ids: &[DataSourceId],
        idempotency_key: &str,
    ) -> Result<IngestionJob, ApplicationError> {
        validate_idempotency_key(idempotency_key)?;
        if data_source_ids.is_empty() || data_source_ids.len() > MAX_SUGGESTION_DATA_SOURCES {
            return Err(ApplicationError::InvalidRequest(format!(
                "an AI suggestion needs 1 to {MAX_SUGGESTION_DATA_SOURCES} data sources"
            )));
        }
        if data_source_ids.iter().collect::<HashSet<_>>().len() != data_source_ids.len() {
            return Err(ApplicationError::InvalidRequest(
                "the data sources of an AI suggestion must be distinct".to_owned(),
            ));
        }
        let mut records = Vec::with_capacity(data_source_ids.len());
        for &id in data_source_ids {
            records.push(self.ingestion.record(graph_id, id).await?);
        }
        let metadata = self.ingestion.metadata();
        if let Some(existing) = metadata
            .find_job_by_idempotency_key(graph_id, idempotency_key)
            .await?
        {
            return replay(existing, data_source_ids);
        }
        if records.iter().any(|record| {
            record.data_source.status() != DataSourceStatus::Ready || record.profile.is_none()
        }) {
            return Err(ApplicationError::Conflict(
                "every data source must be analysed successfully before an AI suggestion"
                    .to_owned(),
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
            data_source_ids.to_vec(),
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
                return replay(existing, data_source_ids);
            }
            Err(other) => return Err(other),
        }
        info!(
            job_id = %job.id,
            graph_id = %graph_id,
            data_source_count = data_source_ids.len(),
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
    data_source_ids: &[DataSourceId],
) -> Result<IngestionJob, ApplicationError> {
    if existing.kind == JobKind::AiSuggest && existing.data_source_ids == data_source_ids {
        Ok(existing)
    } else {
        Err(ApplicationError::Conflict(
            "this Idempotency-Key was already used for a different request".to_owned(),
        ))
    }
}

/// The profile as sent to the AI: at most [`AI_SAMPLE_ROWS`] sample rows,
/// fewer (then fewer sample values) if the serialised profile would exceed
/// `budget` bytes.
fn ai_profile(profile: &DatasetProfile, budget: usize) -> DatasetProfile {
    let mut profile = profile.clone();
    profile.sample_rows.truncate(AI_SAMPLE_ROWS);
    let size = |p: &DatasetProfile| serde_json::to_vec(p).map_or(usize::MAX, |bytes| bytes.len());
    while size(&profile) > budget && !profile.sample_rows.is_empty() {
        profile.sample_rows.pop();
    }
    for keep in [1, 0] {
        if size(&profile) <= budget {
            break;
        }
        for field in &mut profile.fields {
            field.sample_values.truncate(keep);
        }
    }
    profile
}

/// The byte budget of each of `count` profiles in one request.
fn profile_budget(count: usize) -> usize {
    (MAX_AI_PROFILES_BYTES / count.max(1)).min(MAX_AI_PROFILE_BYTES)
}

/// Labels for the AI: the filenames, made unique with a ` (n)` suffix.
fn dataset_labels(records: &[DataSourceRecord]) -> Vec<String> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    records
        .iter()
        .map(|record| {
            let name = record.data_source.filename();
            let count = seen.entry(name).or_default();
            *count += 1;
            if *count == 1 {
                name.to_owned()
            } else {
                format!("{name} ({count})")
            }
        })
        .collect()
}

/// One dataset of a suggestion, for validation.
#[derive(Debug, Clone, Copy)]
pub struct SuggestionDataset<'a> {
    /// The label the AI refers to.
    pub name: &'a str,
    /// The data source behind the label.
    pub data_source_id: DataSourceId,
    /// Its full profile (field names are checked against it).
    pub profile: &'a DatasetProfile,
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
/// ontology and the datasets' profiles. Returns the normalised suggestion
/// (bounded texts, `existing` flags set by Rust, data source ids set and
/// mappings in a safe ingestion order) or every problem found, phrased for a
/// repair prompt: no internal details.
///
/// # Errors
///
/// The validation errors.
pub fn validate_suggestion(
    suggestion: &GraphBuilderSuggestion,
    base: &Ontology,
    datasets: &[SuggestionDataset<'_>],
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
    check_dataset_coverage(&suggestion.mappings, datasets, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }

    let candidate = match ontology.build_next_version(base) {
        Ok(candidate) => candidate,
        Err(err) => return Err(vec![format!("invalid ontology: {err}")]),
    };
    check_preserves_base(base, &candidate, &mut errors);
    for dataset in datasets {
        let Some(mapping) = suggestion
            .mappings
            .iter()
            .find(|mapping| mapping.dataset == dataset.name)
        else {
            continue;
        };
        if let Err(err) =
            validate_mapping(&mapping.mapping, &candidate, &dataset.profile.field_names())
        {
            errors.push(format!("dataset `{}`: {err}", dataset.name));
        }
    }
    check_matched_types_exist(&suggestion.mappings, base, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    let mappings = match ingestion_order(&suggestion.mappings, datasets) {
        Ok(mappings) => mappings,
        Err(error) => return Err(vec![error]),
    };

    let mut normalised = suggestion.clone();
    normalised.mappings = mappings;
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

/// Exactly one mapping per dataset, and no mapping of an unknown dataset.
fn check_dataset_coverage(
    mappings: &[DatasetMapping],
    datasets: &[SuggestionDataset<'_>],
    errors: &mut Vec<String>,
) {
    let mut seen = HashSet::new();
    for mapping in mappings {
        if !datasets
            .iter()
            .any(|dataset| dataset.name == mapping.dataset)
        {
            errors.push(format!(
                "`{}` is not one of the datasets; use their exact names",
                mapping.dataset
            ));
        } else if !seen.insert(mapping.dataset.as_str()) {
            errors.push(format!(
                "dataset `{}` has more than one mapping",
                mapping.dataset
            ));
        }
    }
    for dataset in datasets {
        if !seen.contains(dataset.name) {
            errors.push(format!("dataset `{}` needs a mapping", dataset.name));
        }
    }
}

/// A `match_only` entity type must already exist in the base ontology or be
/// created (`create_or_match`) by one of the datasets; otherwise nothing can
/// ever match.
fn check_matched_types_exist(
    mappings: &[DatasetMapping],
    base: &Ontology,
    errors: &mut Vec<String>,
) {
    let created: HashSet<&str> = mappings
        .iter()
        .flat_map(|mapping| &mapping.mapping.entities)
        .filter(|entity| entity.mode == EntityMode::CreateOrMatch)
        .map(|entity| entity.entity_type.as_str())
        .collect();
    let mut reported = HashSet::new();
    for entity in mappings
        .iter()
        .flat_map(|mapping| &mapping.mapping.entities)
    {
        let name = entity.entity_type.as_str();
        if entity.mode == EntityMode::MatchOnly
            && base.entity_type(name).is_none()
            && !created.contains(name)
            && reported.insert(name)
        {
            errors.push(format!(
                "entity type `{name}` is only matched (match_only) but neither exists yet nor \
                 is created by any dataset; let one dataset create it (create_or_match)"
            ));
        }
    }
}

/// The mappings in an order in which every dataset that only matches an
/// entity type comes after the datasets that create it, ties in the
/// client's dataset order; each with its data source id set.
fn ingestion_order(
    mappings: &[DatasetMapping],
    datasets: &[SuggestionDataset<'_>],
) -> Result<Vec<DatasetMapping>, String> {
    let ordered: Vec<(&SuggestionDataset<'_>, &DatasetMapping)> = datasets
        .iter()
        .filter_map(|dataset| {
            mappings
                .iter()
                .find(|mapping| mapping.dataset == dataset.name)
                .map(|mapping| (dataset, mapping))
        })
        .collect();
    let types_with = |mapping: &DatasetMapping, mode: EntityMode| -> HashSet<String> {
        mapping
            .mapping
            .entities
            .iter()
            .filter(|entity| entity.mode == mode)
            .map(|entity| entity.entity_type.clone())
            .collect()
    };
    let creates: Vec<HashSet<String>> = ordered
        .iter()
        .map(|(_, mapping)| types_with(mapping, EntityMode::CreateOrMatch))
        .collect();
    let matches: Vec<HashSet<String>> = ordered
        .iter()
        .map(|(_, mapping)| types_with(mapping, EntityMode::MatchOnly))
        .collect();
    // `i` must run before `j` when `i` creates a type `j` only matches.
    let depends = |j: usize, i: usize| i != j && !creates[i].is_disjoint(&matches[j]);
    let mut done = vec![false; ordered.len()];
    let mut result = Vec::with_capacity(ordered.len());
    while result.len() < ordered.len() {
        let next = (0..ordered.len())
            .find(|&j| !done[j] && (0..ordered.len()).all(|i| done[i] || !depends(j, i)));
        let Some(j) = next else {
            let stuck: Vec<&str> = (0..ordered.len())
                .filter(|&j| !done[j])
                .map(|j| ordered[j].0.name)
                .collect();
            return Err(format!(
                "datasets {} only match entity types that each other creates; let one of                  them create the type (mode create_or_match)",
                stuck
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        done[j] = true;
        let (dataset, mapping) = ordered[j];
        result.push(DatasetMapping {
            dataset: dataset.name.to_owned(),
            data_source_id: Some(dataset.data_source_id),
            mapping: mapping.mapping.clone(),
        });
    }
    Ok(result)
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

/// The validation errors as the AI service accepts them in a repair
/// request: at most [`MAX_REPAIR_ERRORS`] errors of at most
/// [`MAX_REPAIR_ERROR_CHARS`] characters, the last one counting the rest.
fn repair_errors(errors: Vec<String>) -> Vec<String> {
    let clip = |text: &str| -> String { text.chars().take(MAX_REPAIR_ERROR_CHARS).collect() };
    if errors.len() <= MAX_REPAIR_ERRORS {
        return errors.iter().map(|error| clip(error)).collect();
    }
    let kept = MAX_REPAIR_ERRORS - 1;
    let mut bounded: Vec<String> = errors[..kept].iter().map(|error| clip(error)).collect();
    bounded.push(format!(
        "and {} more problems of the same kinds",
        errors.len() - kept
    ));
    bounded
}

/// Whether the result fits the job store's result bound.
fn result_fits(result: &GraphBuilderResult) -> bool {
    serde_json::to_vec(result).is_ok_and(|bytes| bytes.len() <= MAX_RESULT_BYTES)
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
        let mut records = Vec::with_capacity(job.data_source_ids.len());
        for &id in &job.data_source_ids {
            let record = self
                .metadata
                .get_data_source(job.graph_id, id)
                .await
                .map_err(|err| metadata_failure(&err))?
                .filter(|record| record.profile.is_some())
                .ok_or_else(|| {
                    JobError::new("DATA_SOURCE_NOT_READY", "a data source has no profile")
                })?;
            records.push(record);
        }
        let labels = dataset_labels(&records);
        let profiles: Vec<&DatasetProfile> = records
            .iter()
            .filter_map(|record| record.profile.as_ref())
            .collect();
        let datasets: Vec<SuggestionDataset<'_>> = labels
            .iter()
            .zip(&records)
            .zip(&profiles)
            .map(|((name, record), profile)| SuggestionDataset {
                name,
                data_source_id: record.data_source.id(),
                profile,
            })
            .collect();
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
        let budget = profile_budget(datasets.len());
        let mut request = GraphBuilderRequest {
            correlation_id: job.id.to_string(),
            datasets: datasets
                .iter()
                .map(|dataset| DatasetContext {
                    name: dataset.name.to_owned(),
                    profile: ai_profile(dataset.profile, budget),
                })
                .collect(),
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
            match validate_suggestion(&response.suggestion, &base, &datasets) {
                Ok(suggestion) => {
                    info!(
                        job_id = %job.id,
                        graph_id = %job.graph_id,
                        dataset_count = datasets.len(),
                        provider = %response.metadata.provider,
                        model = %response.metadata.model,
                        prompt_version = %response.metadata.prompt_version,
                        field_count = profiles.iter().map(|p| p.fields.len()).sum::<usize>(),
                        record_count = profiles.iter().map(|p| p.record_count).sum::<u64>(),
                        repair_attempts,
                        duration_ms =
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        "AI suggestion validated"
                    );
                    let result = GraphBuilderResult {
                        prompt_version: response.metadata.prompt_version,
                        provider: response.metadata.provider,
                        model: response.metadata.model,
                        ontology_version_base: version.get(),
                        repair_attempts,
                        suggestion,
                    };
                    if !result_fits(&result) {
                        return Err(JobError::new(
                            "AI_RESULT_TOO_LARGE",
                            "the AI suggestion is too large to store; try a dataset with fewer fields",
                        ));
                    }
                    return Ok(Some(JobResult::GraphBuilder(Box::new(result))));
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
                        errors: repair_errors(errors),
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
        let sent = ai_profile(&profile, MAX_AI_PROFILE_BYTES);
        assert!(sent.sample_rows.len() <= AI_SAMPLE_ROWS);
        assert!(serde_json::to_vec(&sent).unwrap().len() <= MAX_AI_PROFILE_BYTES);
        assert_eq!(sent.record_count, 30);
        let small = ai_profile(&profile, profile_budget(MAX_SUGGESTION_DATA_SOURCES));
        assert!(
            serde_json::to_vec(&small).unwrap().len()
                <= MAX_AI_PROFILES_BYTES / MAX_SUGGESTION_DATA_SOURCES
        );
    }

    #[test]
    fn repair_errors_fit_the_ai_service_schema() {
        let few = vec!["a".repeat(600), "b".to_owned()];
        let sent = repair_errors(few);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].chars().count(), MAX_REPAIR_ERROR_CHARS);
        assert_eq!(sent[1], "b");

        let many: Vec<String> = (0..120).map(|i| format!("error {i}")).collect();
        let sent = repair_errors(many);
        assert_eq!(sent.len(), MAX_REPAIR_ERRORS);
        assert_eq!(sent[48], "error 48");
        assert_eq!(sent[49], "and 71 more problems of the same kinds");
    }

    fn mapping(dataset: &str, entities: &[(&str, EntityMode)]) -> DatasetMapping {
        let entities = entities
            .iter()
            .map(|(entity_type, mode)| {
                serde_json::json!({
                    "alias": entity_type.to_lowercase(),
                    "entity_type": entity_type,
                    "mode": mode,
                    "identity": [{"source_field": "id", "property": "id"}],
                })
            })
            .collect::<Vec<_>>();
        serde_json::from_value(serde_json::json!({
            "dataset": dataset,
            "mapping": {"entities": entities, "relationships": []},
        }))
        .unwrap()
    }

    #[test]
    fn ingestion_order_puts_creators_first_and_reports_cycles() {
        use EntityMode::{CreateOrMatch, MatchOnly};
        let profile = DatasetProfile {
            record_count: 0,
            fields: vec![],
            sample_rows: vec![],
        };
        let ids = [
            DataSourceId::new(),
            DataSourceId::new(),
            DataSourceId::new(),
        ];
        let names = ["devices.csv", "customers.csv", "notes.csv"];
        let datasets: Vec<SuggestionDataset<'_>> = names
            .iter()
            .zip(ids)
            .map(|(name, id)| SuggestionDataset {
                name,
                data_source_id: id,
                profile: &profile,
            })
            .collect();
        let mappings = vec![
            mapping("notes.csv", &[("Note", CreateOrMatch)]),
            mapping(
                "devices.csv",
                &[("Device", CreateOrMatch), ("Customer", MatchOnly)],
            ),
            mapping("customers.csv", &[("Customer", CreateOrMatch)]),
        ];
        let ordered = ingestion_order(&mappings, &datasets).unwrap();
        let order: Vec<(&str, Option<DataSourceId>)> = ordered
            .iter()
            .map(|m| (m.dataset.as_str(), m.data_source_id))
            .collect();
        assert_eq!(
            order,
            [
                ("customers.csv", Some(ids[1])),
                ("devices.csv", Some(ids[0])),
                ("notes.csv", Some(ids[2])),
            ],
            "dependencies first, otherwise the client's order"
        );

        let cycle = vec![
            mapping(
                "devices.csv",
                &[("Device", CreateOrMatch), ("Customer", MatchOnly)],
            ),
            mapping(
                "customers.csv",
                &[("Customer", CreateOrMatch), ("Device", MatchOnly)],
            ),
            mapping("notes.csv", &[("Note", CreateOrMatch)]),
        ];
        let error = ingestion_order(&cycle, &datasets).unwrap_err();
        assert!(
            error.contains("`devices.csv`, `customers.csv`") && !error.contains("notes.csv"),
            "{error}"
        );
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
