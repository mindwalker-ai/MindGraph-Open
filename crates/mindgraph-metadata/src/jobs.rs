//! Data sources and jobs in PostgreSQL (M4).
//!
//! The JSON columns hold the public contract types of `mindgraph-ingestion`
//! (`DatasetProfile`, `IngestionMapping`, `IngestionSummary`) and of
//! `mindgraph-core` (`GraphBuilderResult`), which reject
//! unknown fields, so a stored document that no longer matches is reported
//! as corrupt instead of being half-read.

use chrono::{DateTime, SubsecRound, Utc};
use mindgraph_core::{
    DataSourceRecord, IngestionJob, JobError, JobKind, JobStatus, MetadataStore, MetadataStoreError,
};
use mindgraph_core::{GraphBuilderResult, JobResult};
use mindgraph_domain::{
    DataSource, DataSourceId, GraphId, GraphProject, GraphStatus, IngestionJobId,
};
use mindgraph_ingestion::{DatasetProfile, IngestionMapping, IngestionSummary};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::PostgresMetadataStore;
use crate::store::{corrupt, map_error, version_column, version_from_column};

macro_rules! source_columns {
    () => {
        "id, graph_id, kind, filename, status, storage_key, size_bytes, content_sha256, \
         profile_json, created_at, updated_at"
    };
}

macro_rules! job_columns {
    () => {
        "id, graph_id, data_source_id, kind, status, progress, ontology_version, \
         idempotency_key, mapping_json, error_code, error_message, result_json, \
         created_at, updated_at, \
         ARRAY(SELECT s.data_source_id FROM ingestion_job_data_sources s \
               WHERE s.job_id = ingestion_jobs.id ORDER BY s.position) AS data_source_ids"
    };
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn to_json(what: &str, value: &impl Serialize) -> Result<Value, MetadataStoreError> {
    serde_json::to_value(value)
        .map_err(|err| MetadataStoreError::Serialization(format!("{what}: {err}")))
}

fn from_json<T: DeserializeOwned>(what: &str, value: Value) -> Result<T, MetadataStoreError> {
    serde_json::from_value(value).map_err(|err| corrupt(what, err))
}

/// The constraint a database error names, if any.
fn constraint(err: &sqlx::Error) -> Option<&str> {
    match err {
        sqlx::Error::Database(db) => db.constraint(),
        _ => None,
    }
}

fn decode_source(row: &PgRow) -> Result<DataSourceRecord, MetadataStoreError> {
    let read = |err| map_error("reading a data source", err);
    let id: Uuid = row.try_get("id").map_err(read)?;
    let graph_id: Uuid = row.try_get("graph_id").map_err(read)?;
    let kind: String = row.try_get("kind").map_err(read)?;
    let filename: String = row.try_get("filename").map_err(read)?;
    let status: String = row.try_get("status").map_err(read)?;
    let storage_key: String = row.try_get("storage_key").map_err(read)?;
    let size_bytes: i64 = row.try_get("size_bytes").map_err(read)?;
    let content_sha256: String = row.try_get("content_sha256").map_err(read)?;
    let profile: Option<Value> = row.try_get("profile_json").map_err(read)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(read)?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at").map_err(read)?;
    let data_source = DataSource::from_parts(
        DataSourceId::from_uuid(id).map_err(|err| corrupt("data source id", err))?,
        GraphId::from_uuid(graph_id).map_err(|err| corrupt("graph id", err))?,
        kind.parse()
            .map_err(|err| corrupt("data source kind", err))?,
        &filename,
        status
            .parse()
            .map_err(|err| corrupt("data source status", err))?,
        u64::try_from(size_bytes).map_err(|err| corrupt("data source size", err))?,
        &content_sha256,
        created_at,
        updated_at,
    )
    .map_err(|err| corrupt("data source", err))?;
    Ok(DataSourceRecord {
        data_source,
        storage_key,
        profile: profile
            .map(|value| from_json::<DatasetProfile>("dataset profile", value))
            .transpose()?,
    })
}

/// `result_json` holds the inner value; the job kind says which type it is.
fn decode_result(kind: JobKind, value: Value) -> Result<JobResult, MetadataStoreError> {
    match kind {
        JobKind::Ingest => Ok(JobResult::Ingestion(from_json::<IngestionSummary>(
            "job result",
            value,
        )?)),
        JobKind::AiSuggest => Ok(JobResult::GraphBuilder(Box::new(from_json::<
            GraphBuilderResult,
        >(
            "AI suggestion result",
            value,
        )?))),
        JobKind::Analyze => Err(corrupt("job result", "analysis jobs have no result")),
    }
}

fn encode_result(result: &JobResult) -> Result<Value, MetadataStoreError> {
    match result {
        JobResult::Ingestion(summary) => to_json("job result", summary),
        JobResult::GraphBuilder(result) => to_json("AI suggestion result", result.as_ref()),
    }
}

fn decode_job(row: &PgRow) -> Result<IngestionJob, MetadataStoreError> {
    let read = |err| map_error("reading a job", err);
    let id: Uuid = row.try_get("id").map_err(read)?;
    let graph_id: Uuid = row.try_get("graph_id").map_err(read)?;
    let data_source_id: Uuid = row.try_get("data_source_id").map_err(read)?;
    let listed_sources: Vec<Uuid> = row.try_get("data_source_ids").map_err(read)?;
    let kind: String = row.try_get("kind").map_err(read)?;
    let status: String = row.try_get("status").map_err(read)?;
    let progress: f64 = row.try_get("progress").map_err(read)?;
    let ontology_version: Option<i32> = row.try_get("ontology_version").map_err(read)?;
    let idempotency_key: Option<String> = row.try_get("idempotency_key").map_err(read)?;
    let mapping: Option<Value> = row.try_get("mapping_json").map_err(read)?;
    let error_code: Option<String> = row.try_get("error_code").map_err(read)?;
    let error_message: Option<String> = row.try_get("error_message").map_err(read)?;
    let result: Option<Value> = row.try_get("result_json").map_err(read)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(read)?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at").map_err(read)?;
    let error = match (error_code, error_message) {
        (Some(code), Some(message)) => Some(JobError { code, message }),
        (None, None) => None,
        _ => {
            return Err(corrupt(
                "job error",
                "code and message must be set together",
            ));
        }
    };
    let kind = kind
        .parse::<JobKind>()
        .map_err(|err| corrupt("job kind", err))?;
    let data_source_id =
        DataSourceId::from_uuid(data_source_id).map_err(|err| corrupt("data source id", err))?;
    // Jobs of one data source have no listed sources (including AI jobs
    // created before multi-dataset suggestions).
    let data_source_ids = if listed_sources.is_empty() {
        vec![data_source_id]
    } else {
        listed_sources
            .into_iter()
            .map(|id| DataSourceId::from_uuid(id).map_err(|err| corrupt("data source id", err)))
            .collect::<Result<Vec<_>, _>>()?
    };
    if data_source_ids.first() != Some(&data_source_id) {
        return Err(corrupt(
            "job data sources",
            "the first listed data source must be the job's data source",
        ));
    }
    let mut result = result.map(|value| decode_result(kind, value)).transpose()?;
    // Suggestions stored before multi-dataset support have one mapping
    // without a data source: it is the job's data source.
    if let Some(JobResult::GraphBuilder(stored)) = &mut result {
        for mapping in &mut stored.suggestion.mappings {
            mapping.data_source_id.get_or_insert(data_source_id);
        }
    }
    Ok(IngestionJob {
        id: IngestionJobId::from_uuid(id).map_err(|err| corrupt("job id", err))?,
        graph_id: GraphId::from_uuid(graph_id).map_err(|err| corrupt("graph id", err))?,
        data_source_id,
        data_source_ids,
        kind,
        status: status
            .parse::<JobStatus>()
            .map_err(|err| corrupt("job status", err))?,
        progress,
        ontology_version: ontology_version.map(version_from_column).transpose()?,
        idempotency_key,
        mapping: mapping
            .map(|value| from_json::<IngestionMapping>("ingestion mapping", value))
            .transpose()?,
        error,
        result,
        created_at,
        updated_at,
    })
}

impl PostgresMetadataStore {
    pub(crate) async fn activate_graph_row(
        &self,
        id: GraphId,
    ) -> Result<GraphProject, MetadataStoreError> {
        // Only draft → active; active stays, archived is not reactivated.
        sqlx::query(
            "UPDATE graph_projects SET status = $2, updated_at = GREATEST(updated_at, $3) \
             WHERE id = $1 AND status = $4",
        )
        .bind(id.as_uuid())
        .bind(GraphStatus::Active.as_str())
        .bind(now())
        .bind(GraphStatus::Draft.as_str())
        .execute(self.pool())
        .await
        .map_err(|err| map_error("activating a graph project", err))?;
        self.get_graph(id)
            .await?
            .ok_or_else(|| MetadataStoreError::NotFound(format!("graph {id}")))
    }

    pub(crate) async fn insert_data_source(
        &self,
        source: &DataSource,
        storage_key: &str,
    ) -> Result<(), MetadataStoreError> {
        let size = i64::try_from(source.size_bytes()).map_err(|_| {
            MetadataStoreError::InvalidInput("the data source is too large".to_owned())
        })?;
        sqlx::query(
            "INSERT INTO data_sources \
             (id, graph_id, kind, filename, status, storage_key, size_bytes, content_sha256, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(source.id().as_uuid())
        .bind(source.graph_id().as_uuid())
        .bind(source.kind().as_str())
        .bind(source.filename())
        .bind(source.status().as_str())
        .bind(storage_key)
        .bind(size)
        .bind(source.content_sha256())
        .bind(source.created_at())
        .bind(source.updated_at())
        .execute(self.pool())
        .await
        .map(drop)
        .map_err(|err| match map_error("creating a data source", err) {
            MetadataStoreError::NotFound(_) => {
                MetadataStoreError::NotFound(format!("graph {}", source.graph_id()))
            }
            MetadataStoreError::Conflict(_) => {
                MetadataStoreError::Conflict(format!("data source {} already exists", source.id()))
            }
            other => other,
        })
    }

    pub(crate) async fn select_data_source(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<Option<DataSourceRecord>, MetadataStoreError> {
        sqlx::query(concat!(
            "SELECT ",
            source_columns!(),
            " FROM data_sources WHERE id = $1 AND graph_id = $2"
        ))
        .bind(id.as_uuid())
        .bind(graph_id.as_uuid())
        .fetch_optional(self.pool())
        .await
        .map_err(|err| map_error("reading a data source", err))?
        .as_ref()
        .map(decode_source)
        .transpose()
    }

    pub(crate) async fn set_data_source_profile(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
        profile: Option<&DatasetProfile>,
    ) -> Result<DataSource, MetadataStoreError> {
        let (status, profile_json, record_count) = match profile {
            Some(profile) => (
                "ready",
                Some(to_json("dataset profile", profile)?),
                Some(i64::try_from(profile.record_count).map_err(|_| {
                    MetadataStoreError::InvalidInput("the record count is too large".to_owned())
                })?),
            ),
            None => ("failed", None, None),
        };
        sqlx::query(concat!(
            "UPDATE data_sources SET status = $3, profile_json = $4, record_count = $5, \
             updated_at = GREATEST(updated_at, $6) WHERE id = $1 AND graph_id = $2 RETURNING ",
            source_columns!()
        ))
        .bind(id.as_uuid())
        .bind(graph_id.as_uuid())
        .bind(status)
        .bind(profile_json)
        .bind(record_count)
        .bind(now())
        .fetch_optional(self.pool())
        .await
        .map_err(|err| map_error("updating a data source", err))?
        .as_ref()
        .map(decode_source)
        .transpose()?
        .map(|record| record.data_source)
        .ok_or_else(|| {
            MetadataStoreError::NotFound(format!("data source {id} of graph {graph_id}"))
        })
    }

    pub(crate) async fn insert_job(&self, job: &IngestionJob) -> Result<(), MetadataStoreError> {
        job.validate_new()
            .map_err(MetadataStoreError::InvalidInput)?;
        let version = job.ontology_version.map(version_column).transpose()?;
        let mapping = job
            .mapping
            .as_ref()
            .map(|mapping| to_json("ingestion mapping", mapping))
            .transpose()?;
        let result = self.insert_job_rows(job, version, mapping).await;
        match result {
            Ok(()) => Ok(()),
            Err(err) => {
                let pinned = constraint(&err) == Some("ingestion_jobs_pinned_ontology_fk");
                Err(match map_error("creating a job", err) {
                    MetadataStoreError::NotFound(_) if pinned => {
                        MetadataStoreError::InvalidInput(format!(
                            "the pinned ontology version of job {} does not exist",
                            job.id
                        ))
                    }
                    MetadataStoreError::NotFound(_) => MetadataStoreError::NotFound(format!(
                        "a data source of job {} in graph {}",
                        job.id, job.graph_id
                    )),
                    MetadataStoreError::Conflict(_) => MetadataStoreError::Conflict(format!(
                        "job {} or its idempotency key already exists",
                        job.id
                    )),
                    other => other,
                })
            }
        }
    }

    /// The job row and, for AI suggestions, its listed data sources, in one
    /// transaction.
    async fn insert_job_rows(
        &self,
        job: &IngestionJob,
        version: Option<i32>,
        mapping: Option<Value>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "INSERT INTO ingestion_jobs \
             (id, graph_id, data_source_id, kind, status, progress, ontology_version, \
              idempotency_key, mapping_json, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(job.id.as_uuid())
        .bind(job.graph_id.as_uuid())
        .bind(job.data_source_id.as_uuid())
        .bind(job.kind.as_str())
        .bind(job.status.as_str())
        .bind(job.progress)
        .bind(version)
        .bind(job.idempotency_key.as_deref())
        .bind(mapping)
        .bind(job.created_at)
        .bind(job.updated_at)
        .execute(&mut *tx)
        .await?;
        if job.kind == JobKind::AiSuggest {
            let sources: Vec<Uuid> = job.data_source_ids.iter().map(|id| id.as_uuid()).collect();
            sqlx::query(
                "INSERT INTO ingestion_job_data_sources (job_id, graph_id, position, data_source_id) \
                 SELECT $1, $2, (t.ord - 1)::smallint, t.id \
                 FROM UNNEST($3::uuid[]) WITH ORDINALITY AS t(id, ord)",
            )
            .bind(job.id.as_uuid())
            .bind(job.graph_id.as_uuid())
            .bind(&sources)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }

    pub(crate) async fn select_job(
        &self,
        id: IngestionJobId,
    ) -> Result<Option<IngestionJob>, MetadataStoreError> {
        sqlx::query(concat!(
            "SELECT ",
            job_columns!(),
            " FROM ingestion_jobs WHERE id = $1"
        ))
        .bind(id.as_uuid())
        .fetch_optional(self.pool())
        .await
        .map_err(|err| map_error("reading a job", err))?
        .as_ref()
        .map(decode_job)
        .transpose()
    }

    pub(crate) async fn select_job_by_key(
        &self,
        graph_id: GraphId,
        key: &str,
    ) -> Result<Option<IngestionJob>, MetadataStoreError> {
        sqlx::query(concat!(
            "SELECT ",
            job_columns!(),
            " FROM ingestion_jobs WHERE graph_id = $1 AND idempotency_key = $2"
        ))
        .bind(graph_id.as_uuid())
        .bind(key)
        .fetch_optional(self.pool())
        .await
        .map_err(|err| map_error("reading a job", err))?
        .as_ref()
        .map(decode_job)
        .transpose()
    }

    /// Explains why a guarded job update matched no row.
    async fn transition_refused(&self, id: IngestionJobId, action: &str) -> MetadataStoreError {
        match self.select_job(id).await {
            Ok(Some(job)) => MetadataStoreError::Conflict(format!(
                "cannot {action} job {id}: it is {}",
                job.status
            )),
            Ok(None) => MetadataStoreError::NotFound(format!("job {id}")),
            Err(err) => err,
        }
    }

    async fn guarded_update(
        &self,
        id: IngestionJobId,
        action: &str,
        row: Option<PgRow>,
    ) -> Result<IngestionJob, MetadataStoreError> {
        match row {
            Some(row) => decode_job(&row),
            None => Err(self.transition_refused(id, action).await),
        }
    }

    pub(crate) async fn start_job_row(
        &self,
        id: IngestionJobId,
    ) -> Result<IngestionJob, MetadataStoreError> {
        let row = sqlx::query(concat!(
            "UPDATE ingestion_jobs SET status = 'running', updated_at = GREATEST(updated_at, $2) \
             WHERE id = $1 AND status = 'queued' RETURNING ",
            job_columns!()
        ))
        .bind(id.as_uuid())
        .bind(now())
        .fetch_optional(self.pool())
        .await
        .map_err(|err| map_error("starting a job", err))?;
        self.guarded_update(id, "start", row).await
    }

    pub(crate) async fn update_progress_row(
        &self,
        id: IngestionJobId,
        progress: f64,
    ) -> Result<(), MetadataStoreError> {
        if progress.is_nan() {
            return Err(MetadataStoreError::InvalidInput(
                "progress must be a number".to_owned(),
            ));
        }
        let progress = progress.clamp(0.0, 1.0);
        let row = sqlx::query(concat!(
            "UPDATE ingestion_jobs SET progress = GREATEST(progress, $2), \
             updated_at = CASE WHEN $2 > progress THEN GREATEST(updated_at, $3) ELSE updated_at END \
             WHERE id = $1 AND status = 'running' RETURNING ",
            job_columns!()
        ))
        .bind(id.as_uuid())
        .bind(progress)
        .bind(now())
        .fetch_optional(self.pool())
        .await
        .map_err(|err| map_error("updating job progress", err))?;
        self.guarded_update(id, "update progress of", row)
            .await
            .map(drop)
    }

    pub(crate) async fn complete_job_row(
        &self,
        id: IngestionJobId,
        result: Option<&JobResult>,
    ) -> Result<IngestionJob, MetadataStoreError> {
        if let Some(result) = result
            && let Some(job) = self.select_job(id).await?
            && job.kind != result.kind()
        {
            return Err(MetadataStoreError::InvalidInput(format!(
                "a {} result cannot complete {} job {id}",
                result.kind(),
                job.kind
            )));
        }
        let result = result.map(encode_result).transpose()?;
        let row = sqlx::query(concat!(
            "UPDATE ingestion_jobs SET status = 'completed', progress = 1, result_json = $2, \
             updated_at = GREATEST(updated_at, $3) WHERE id = $1 AND status = 'running' RETURNING ",
            job_columns!()
        ))
        .bind(id.as_uuid())
        .bind(result)
        .bind(now())
        .fetch_optional(self.pool())
        .await
        .map_err(|err| map_error("completing a job", err))?;
        self.guarded_update(id, "complete", row).await
    }

    pub(crate) async fn fail_job_row(
        &self,
        id: IngestionJobId,
        error: &JobError,
    ) -> Result<IngestionJob, MetadataStoreError> {
        let row = sqlx::query(concat!(
            "UPDATE ingestion_jobs SET status = 'failed', error_code = $2, error_message = $3, \
             updated_at = GREATEST(updated_at, $4) \
             WHERE id = $1 AND status IN ('queued', 'running') RETURNING ",
            job_columns!()
        ))
        .bind(id.as_uuid())
        .bind(&error.code)
        .bind(&error.message)
        .bind(now())
        .fetch_optional(self.pool())
        .await
        .map_err(|err| map_error("failing a job", err))?;
        self.guarded_update(id, "fail", row).await
    }

    pub(crate) async fn fail_incomplete_jobs(
        &self,
        error: &JobError,
    ) -> Result<u64, MetadataStoreError> {
        sqlx::query(
            "UPDATE ingestion_jobs SET status = 'failed', error_code = $1, error_message = $2, \
             updated_at = GREATEST(updated_at, $3) WHERE status IN ('queued', 'running')",
        )
        .bind(&error.code)
        .bind(&error.message)
        .bind(now())
        .execute(self.pool())
        .await
        .map(|done| done.rows_affected())
        .map_err(|err| map_error("failing incomplete jobs", err))
    }
}
