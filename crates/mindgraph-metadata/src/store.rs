//! `PostgresMetadataStore`: the SQLx implementation of `MetadataStore`.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, SubsecRound, Utc};
use mindgraph_core::{
    DataSourceRecord, IngestionJob, JobError, JobResult, LOCAL_WORKSPACE_NAME, MetadataStore,
    MetadataStoreError,
};
use mindgraph_domain::{
    DataSource, DataSourceId, GraphId, GraphProject, IngestionJobId, Ontology, OntologyVersion,
    Workspace, WorkspaceId,
};
use mindgraph_ingestion::DatasetProfile;
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use uuid::Uuid;

use crate::{MIGRATOR, codec};

/// The `system_key` of the local workspace row.
const LOCAL_SYSTEM_KEY: &str = "local";

/// Connection settings for [`PostgresMetadataStore`].
///
/// The connection URL may contain a password, so `Debug` never prints it.
#[derive(Clone)]
pub struct PostgresOptions {
    url: String,
    max_connections: u32,
    acquire_timeout: Duration,
}

impl PostgresOptions {
    /// The pool size used unless overridden.
    pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;
    /// How long to wait for a connection before giving up.
    pub const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

    /// Options for the database at `url` (`postgres://user:password@host/db`).
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: Self::DEFAULT_MAX_CONNECTIONS,
            acquire_timeout: Self::DEFAULT_ACQUIRE_TIMEOUT,
        }
    }

    /// Sets the maximum number of pooled connections (at least 1).
    #[must_use]
    pub fn with_max_connections(mut self, max_connections: u32) -> Self {
        self.max_connections = max_connections.max(1);
        self
    }

    /// Sets how long to wait for a pooled connection.
    #[must_use]
    pub const fn with_acquire_timeout(mut self, timeout: Duration) -> Self {
        self.acquire_timeout = timeout;
        self
    }
}

impl fmt::Debug for PostgresOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresOptions")
            .field("url", &"<redacted>")
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .finish()
    }
}

/// A [`MetadataStore`] backed by PostgreSQL.
#[derive(Debug, Clone)]
pub struct PostgresMetadataStore {
    pool: PgPool,
}

impl PostgresMetadataStore {
    /// Connects to PostgreSQL and checks one connection, failing fast when
    /// the database is unreachable. No retry loop: the process supervisor
    /// (Docker's restart policy) handles restarts.
    ///
    /// # Errors
    ///
    /// [`MetadataStoreError::BackendUnavailable`] if no connection can be
    /// made within the acquire timeout.
    pub async fn connect(options: &PostgresOptions) -> Result<Self, MetadataStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(options.max_connections)
            .acquire_timeout(options.acquire_timeout)
            .connect(&options.url)
            .await
            .map_err(|err| map_error("connecting to PostgreSQL", err))?;
        Ok(Self { pool })
    }

    /// Wraps an existing pool (used by tests).
    #[must_use]
    pub const fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(crate) const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Applies any pending embedded migrations. Safe to call on every start;
    /// concurrent callers are serialised by SQLx's migration lock.
    ///
    /// # Errors
    ///
    /// [`MetadataStoreError::BackendError`] if a migration fails, or
    /// [`MetadataStoreError::BackendUnavailable`] if the database is
    /// unreachable.
    pub async fn migrate(&self) -> Result<(), MetadataStoreError> {
        MIGRATOR.run(&self.pool).await.map_err(|err| match err {
            sqlx::migrate::MigrateError::Execute(inner) => map_error("running migrations", inner),
            other => MetadataStoreError::BackendError {
                message: "running migrations failed".to_owned(),
                source: Some(Box::new(other)),
            },
        })
    }
}

/// Maps a SQLx error to a store error. Messages never include connection
/// strings or row values.
pub(crate) fn map_error(context: &str, err: sqlx::Error) -> MetadataStoreError {
    enum Kind {
        Conflict,
        NotFound,
        Invalid,
        Unavailable,
        Corrupt,
        Other,
    }
    let (kind, detail) = match &err {
        sqlx::Error::Database(db) => {
            let constraint = db.constraint().unwrap_or("unknown constraint").to_owned();
            match db.code().as_deref() {
                Some("23505") => (Kind::Conflict, constraint),
                Some("23503") => (Kind::NotFound, constraint),
                Some("23502" | "23514" | "22P02" | "23001") => (Kind::Invalid, constraint),
                Some(code)
                    if code.starts_with("08") || code.starts_with("57P") || code == "53300" =>
                {
                    (Kind::Unavailable, code.to_owned())
                }
                Some(code) => (Kind::Other, code.to_owned()),
                None => (Kind::Other, String::new()),
            }
        }
        sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::Io(_)
        | sqlx::Error::Tls(_) => (Kind::Unavailable, String::new()),
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnNotFound(_) => (Kind::Corrupt, err.to_string()),
        _ => (Kind::Other, String::new()),
    };
    match kind {
        Kind::Conflict => {
            MetadataStoreError::Conflict(format!("{context}: already exists ({detail})"))
        }
        Kind::NotFound => MetadataStoreError::NotFound(format!(
            "{context}: referenced record is missing ({detail})"
        )),
        Kind::Invalid => {
            MetadataStoreError::InvalidInput(format!("{context}: rejected by constraint {detail}"))
        }
        Kind::Unavailable => MetadataStoreError::BackendUnavailable {
            message: format!("{context}: PostgreSQL is unreachable"),
            source: Some(Box::new(err)),
        },
        Kind::Corrupt => MetadataStoreError::CorruptData(format!("{context}: {detail}")),
        Kind::Other => MetadataStoreError::BackendError {
            message: format!("{context} failed {detail}").trim_end().to_owned(),
            source: Some(Box::new(err)),
        },
    }
}

pub(crate) fn corrupt(what: &str, err: impl fmt::Display) -> MetadataStoreError {
    MetadataStoreError::CorruptData(format!("{what}: {err}"))
}

pub(crate) fn version_column(version: OntologyVersion) -> Result<i32, MetadataStoreError> {
    i32::try_from(version.get()).map_err(|_| {
        MetadataStoreError::InvalidInput(format!("ontology version {version} is too large"))
    })
}

pub(crate) fn version_from_column(value: i32) -> Result<OntologyVersion, MetadataStoreError> {
    u32::try_from(value)
        .map_err(|_| corrupt("ontology version", value))
        .and_then(|value| {
            OntologyVersion::new(value).map_err(|err| corrupt("ontology version", err))
        })
}

fn decode_workspace(row: &PgRow) -> Result<Workspace, MetadataStoreError> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|err| map_error("reading workspace", err))?;
    let name: String = row
        .try_get("name")
        .map_err(|err| map_error("reading workspace", err))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|err| map_error("reading workspace", err))?;
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|err| map_error("reading workspace", err))?;
    let id = WorkspaceId::from_uuid(id).map_err(|err| corrupt("workspace id", err))?;
    Workspace::from_parts(id, &name, created_at, updated_at)
        .map_err(|err| corrupt("workspace", err))
}

fn decode_graph(row: &PgRow) -> Result<GraphProject, MetadataStoreError> {
    let read = |err| map_error("reading graph project", err);
    let id: Uuid = row.try_get("id").map_err(read)?;
    let workspace_id: Uuid = row.try_get("workspace_id").map_err(read)?;
    let name: String = row.try_get("name").map_err(read)?;
    let description: Option<String> = row.try_get("description").map_err(read)?;
    let status: String = row.try_get("status").map_err(read)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(read)?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at").map_err(read)?;
    GraphProject::from_parts(
        GraphId::from_uuid(id).map_err(|err| corrupt("graph id", err))?,
        WorkspaceId::from_uuid(workspace_id).map_err(|err| corrupt("workspace id", err))?,
        &name,
        description.as_deref(),
        status.parse().map_err(|err| corrupt("graph status", err))?,
        created_at,
        updated_at,
    )
    .map_err(|err| corrupt("graph project", err))
}

#[async_trait]
impl MetadataStore for PostgresMetadataStore {
    async fn ensure_local_workspace(&self) -> Result<Workspace, MetadataStoreError> {
        let now = Utc::now().trunc_subsecs(6);
        // Race-safe and idempotent: concurrent first starts insert at most
        // one row, and later starts change nothing.
        sqlx::query(
            "INSERT INTO workspaces (id, system_key, name, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $4) \
             ON CONFLICT (system_key) DO NOTHING",
        )
        .bind(WorkspaceId::new().as_uuid())
        .bind(LOCAL_SYSTEM_KEY)
        .bind(LOCAL_WORKSPACE_NAME)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|err| map_error("creating the local workspace", err))?;

        let row = sqlx::query(
            "SELECT id, name, created_at, updated_at FROM workspaces WHERE system_key = $1",
        )
        .bind(LOCAL_SYSTEM_KEY)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| map_error("reading the local workspace", err))?;
        decode_workspace(&row)
    }

    async fn get_workspace(
        &self,
        id: WorkspaceId,
    ) -> Result<Option<Workspace>, MetadataStoreError> {
        sqlx::query("SELECT id, name, created_at, updated_at FROM workspaces WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| map_error("reading a workspace", err))?
            .as_ref()
            .map(decode_workspace)
            .transpose()
    }

    async fn create_graph(
        &self,
        graph: &GraphProject,
        initial_ontology: &Ontology,
    ) -> Result<(), MetadataStoreError> {
        if initial_ontology.graph_id() != graph.id() {
            return Err(MetadataStoreError::InvalidInput(format!(
                "initial ontology belongs to graph {}, not {}",
                initial_ontology.graph_id(),
                graph.id()
            )));
        }
        let version = version_column(initial_ontology.version())?;
        let schema = codec::encode(initial_ontology)?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| map_error("starting a transaction", err))?;
        sqlx::query(
            "INSERT INTO graph_projects \
             (id, workspace_id, name, description, status, active_ontology_version, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(graph.id().as_uuid())
        .bind(graph.workspace_id().as_uuid())
        .bind(graph.name())
        .bind(graph.description())
        .bind(graph.status().as_str())
        .bind(version)
        .bind(graph.created_at())
        .bind(graph.updated_at())
        .execute(&mut *tx)
        .await
        .map_err(|err| match map_error("creating the graph project", err) {
            MetadataStoreError::NotFound(_) => {
                MetadataStoreError::NotFound(format!("workspace {}", graph.workspace_id()))
            }
            MetadataStoreError::Conflict(_) => {
                MetadataStoreError::Conflict(format!("graph {} already exists", graph.id()))
            }
            other => other,
        })?;
        sqlx::query(
            "INSERT INTO ontology_versions (graph_id, version, schema_json) VALUES ($1, $2, $3)",
        )
        .bind(graph.id().as_uuid())
        .bind(version)
        .bind(&schema)
        .execute(&mut *tx)
        .await
        .map_err(|err| map_error("creating the initial ontology", err))?;
        // The deferred "active ontology exists" constraint is checked here.
        tx.commit()
            .await
            .map_err(|err| map_error("committing the graph project", err))
        // Dropping an uncommitted transaction rolls it back.
    }

    async fn get_graph(&self, id: GraphId) -> Result<Option<GraphProject>, MetadataStoreError> {
        sqlx::query(
            "SELECT id, workspace_id, name, description, status, created_at, updated_at \
             FROM graph_projects WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_error("reading a graph project", err))?
        .as_ref()
        .map(decode_graph)
        .transpose()
    }

    async fn list_graphs(
        &self,
        workspace_id: WorkspaceId,
        limit: usize,
    ) -> Result<Vec<GraphProject>, MetadataStoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        sqlx::query(
            "SELECT id, workspace_id, name, description, status, created_at, updated_at \
             FROM graph_projects WHERE workspace_id = $1 \
             ORDER BY created_at DESC, id DESC LIMIT $2",
        )
        .bind(workspace_id.as_uuid())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| map_error("listing graph projects", err))?
        .iter()
        .map(decode_graph)
        .collect()
    }

    async fn create_ontology(&self, ontology: &Ontology) -> Result<(), MetadataStoreError> {
        let graph_id = ontology.graph_id();
        let version = ontology.version();
        sqlx::query(
            "INSERT INTO ontology_versions (graph_id, version, schema_json) VALUES ($1, $2, $3)",
        )
        .bind(graph_id.as_uuid())
        .bind(version_column(version)?)
        .bind(codec::encode(ontology)?)
        .execute(&self.pool)
        .await
        .map(drop)
        .map_err(|err| match map_error("creating an ontology version", err) {
            MetadataStoreError::NotFound(_) => {
                MetadataStoreError::NotFound(format!("graph {graph_id}"))
            }
            MetadataStoreError::Conflict(_) => MetadataStoreError::Conflict(format!(
                "ontology {version} of graph {graph_id} already exists"
            )),
            other => other,
        })
    }

    async fn create_and_activate_ontology(
        &self,
        ontology: &Ontology,
    ) -> Result<(), MetadataStoreError> {
        let graph_id = ontology.graph_id();
        let version = ontology.version();
        let column = version_column(version)?;
        let schema = codec::encode(ontology)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| map_error("starting a transaction", err))?;
        sqlx::query(
            "INSERT INTO ontology_versions (graph_id, version, schema_json) VALUES ($1, $2, $3)",
        )
        .bind(graph_id.as_uuid())
        .bind(column)
        .bind(&schema)
        .execute(&mut *tx)
        .await
        .map_err(|err| match map_error("creating an ontology version", err) {
            MetadataStoreError::NotFound(_) => {
                MetadataStoreError::NotFound(format!("graph {graph_id}"))
            }
            MetadataStoreError::Conflict(_) => MetadataStoreError::Conflict(format!(
                "ontology {version} of graph {graph_id} already exists"
            )),
            other => other,
        })?;
        sqlx::query(
            "UPDATE graph_projects \
             SET active_ontology_version = $2, updated_at = GREATEST(updated_at, $3) \
             WHERE id = $1",
        )
        .bind(graph_id.as_uuid())
        .bind(column)
        .bind(Utc::now().trunc_subsecs(6))
        .execute(&mut *tx)
        .await
        .map_err(|err| map_error("activating an ontology version", err))?;
        tx.commit()
            .await
            .map_err(|err| map_error("committing the ontology version", err))
    }

    async fn get_ontology(
        &self,
        graph_id: GraphId,
        version: OntologyVersion,
    ) -> Result<Option<Ontology>, MetadataStoreError> {
        let row = sqlx::query(
            "SELECT schema_json FROM ontology_versions WHERE graph_id = $1 AND version = $2",
        )
        .bind(graph_id.as_uuid())
        .bind(version_column(version)?)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_error("reading an ontology version", err))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let schema: Value = row
            .try_get("schema_json")
            .map_err(|err| map_error("reading an ontology version", err))?;
        codec::decode(graph_id, version, schema).map(Some)
    }

    async fn get_active_ontology(
        &self,
        graph_id: GraphId,
    ) -> Result<Option<Ontology>, MetadataStoreError> {
        let row = sqlx::query(
            "SELECT o.version, o.schema_json \
             FROM graph_projects g \
             JOIN ontology_versions o ON o.graph_id = g.id AND o.version = g.active_ontology_version \
             WHERE g.id = $1",
        )
        .bind(graph_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_error("reading the active ontology", err))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let read = |err| map_error("reading the active ontology", err);
        let version = version_from_column(row.try_get("version").map_err(read)?)?;
        let schema: Value = row.try_get("schema_json").map_err(read)?;
        codec::decode(graph_id, version, schema).map(Some)
    }

    async fn set_active_ontology(
        &self,
        graph_id: GraphId,
        version: OntologyVersion,
    ) -> Result<(), MetadataStoreError> {
        let now = Utc::now().trunc_subsecs(6);
        let result = sqlx::query(
            "UPDATE graph_projects \
             SET active_ontology_version = $2, updated_at = GREATEST(updated_at, $3) \
             WHERE id = $1",
        )
        .bind(graph_id.as_uuid())
        .bind(version_column(version)?)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(
            |err| match map_error("activating an ontology version", err) {
                MetadataStoreError::NotFound(_) => {
                    MetadataStoreError::NotFound(format!("ontology {version} of graph {graph_id}"))
                }
                other => other,
            },
        )?;
        if result.rows_affected() == 0 {
            return Err(MetadataStoreError::NotFound(format!("graph {graph_id}")));
        }
        Ok(())
    }

    async fn activate_graph(&self, id: GraphId) -> Result<GraphProject, MetadataStoreError> {
        self.activate_graph_row(id).await
    }

    async fn create_data_source(
        &self,
        data_source: &DataSource,
        storage_key: &str,
    ) -> Result<(), MetadataStoreError> {
        self.insert_data_source(data_source, storage_key).await
    }

    async fn get_data_source(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<Option<DataSourceRecord>, MetadataStoreError> {
        self.select_data_source(graph_id, id).await
    }

    async fn mark_data_source_ready(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
        profile: &DatasetProfile,
    ) -> Result<DataSource, MetadataStoreError> {
        self.set_data_source_profile(graph_id, id, Some(profile))
            .await
    }

    async fn mark_data_source_failed(
        &self,
        graph_id: GraphId,
        id: DataSourceId,
    ) -> Result<DataSource, MetadataStoreError> {
        self.set_data_source_profile(graph_id, id, None).await
    }

    async fn create_job(&self, job: &IngestionJob) -> Result<(), MetadataStoreError> {
        self.insert_job(job).await
    }

    async fn get_job(
        &self,
        id: IngestionJobId,
    ) -> Result<Option<IngestionJob>, MetadataStoreError> {
        self.select_job(id).await
    }

    async fn find_job_by_idempotency_key(
        &self,
        graph_id: GraphId,
        key: &str,
    ) -> Result<Option<IngestionJob>, MetadataStoreError> {
        self.select_job_by_key(graph_id, key).await
    }

    async fn start_job(&self, id: IngestionJobId) -> Result<IngestionJob, MetadataStoreError> {
        self.start_job_row(id).await
    }

    async fn update_job_progress(
        &self,
        id: IngestionJobId,
        progress: f64,
    ) -> Result<(), MetadataStoreError> {
        self.update_progress_row(id, progress).await
    }

    async fn complete_job(
        &self,
        id: IngestionJobId,
        result: Option<&JobResult>,
    ) -> Result<IngestionJob, MetadataStoreError> {
        self.complete_job_row(id, result).await
    }

    async fn fail_job(
        &self,
        id: IngestionJobId,
        error: &JobError,
    ) -> Result<IngestionJob, MetadataStoreError> {
        self.fail_job_row(id, error).await
    }

    async fn mark_incomplete_jobs_failed(
        &self,
        error: &JobError,
    ) -> Result<u64, MetadataStoreError> {
        self.fail_incomplete_jobs(error).await
    }

    async fn ping(&self) -> Result<(), MetadataStoreError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(drop)
            .map_err(|err| map_error("checking PostgreSQL", err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_connection_url() {
        let options = PostgresOptions::new("postgres://mindgraph:s3cret@db:5432/mindgraph")
            .with_max_connections(4);
        let debug = format!("{options:?}");
        assert!(!debug.contains("s3cret"), "{debug}");
        assert!(debug.contains("<redacted>"));
        assert_eq!(
            PostgresOptions::new("x")
                .with_max_connections(0)
                .max_connections,
            1
        );
    }

    #[test]
    fn versions_convert_to_and_from_columns() {
        let v3 = OntologyVersion::new(3).unwrap();
        assert_eq!(version_column(v3).unwrap(), 3);
        assert_eq!(version_from_column(3).unwrap(), v3);
        assert!(matches!(
            version_from_column(0),
            Err(MetadataStoreError::CorruptData(_))
        ));
        assert!(matches!(
            version_from_column(-1),
            Err(MetadataStoreError::CorruptData(_))
        ));
        assert!(matches!(
            version_column(OntologyVersion::new(u32::MAX).unwrap()),
            Err(MetadataStoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn pool_timeouts_are_unavailable() {
        assert!(matches!(
            map_error("x", sqlx::Error::PoolTimedOut),
            MetadataStoreError::BackendUnavailable { .. }
        ));
        assert!(matches!(
            map_error("x", sqlx::Error::RowNotFound),
            MetadataStoreError::BackendError { .. }
        ));
    }
}
