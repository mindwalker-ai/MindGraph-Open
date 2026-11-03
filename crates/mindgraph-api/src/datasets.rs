//! Handlers for ontology versions, data sources and jobs (M4).
//!
//! Transport only: multipart streaming, headers and SSE framing live here;
//! parsing, validation and graph writes happen in `mindgraph-core` and
//! `mindgraph-ingestion`.

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::multipart::{MultipartError, MultipartRejection};
use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use futures_util::stream;
use mindgraph_core::{IngestionJob, IngestionService, JobStatus};
use mindgraph_domain::{DataSourceId, GraphId, IngestionJobId};

use crate::dto::{
    CreateOntologyRequest, DataSourceResponse, IngestionAcceptedResponse, IngestionRequest,
    JobAcceptedResponse, JobResponse, OntologyResponse,
};
use crate::error::ApiError;
use crate::routes::{AppState, graph_id};

/// The multipart field that carries the file.
const FILE_FIELD: &str = "file";
/// How often the SSE stream re-reads job state.
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// SSE comment keep-alives, so proxies keep idle streams open.
const EVENT_KEEP_ALIVE: Duration = Duration::from_secs(15);

fn data_source_path(
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<(GraphId, DataSourceId), ApiError> {
    let Path((graph, data_source)) = path.map_err(|_| ApiError::invalid_graph_id())?;
    let graph = graph.parse().map_err(|_| ApiError::invalid_graph_id())?;
    let data_source = data_source
        .parse()
        .map_err(|_| ApiError::invalid_id("INVALID_DATA_SOURCE_ID", "data source id"))?;
    Ok((graph, data_source))
}

fn job_path(path: Result<Path<String>, PathRejection>) -> Result<IngestionJobId, ApiError> {
    let invalid = || ApiError::invalid_id("INVALID_JOB_ID", "job id");
    let Path(raw) = path.map_err(|_| invalid())?;
    raw.parse().map_err(|_| invalid())
}

fn location(path: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(path) {
        headers.insert(header::LOCATION, value);
    }
    headers
}

/// `POST /api/v1/graphs/{graph_id}/ontology`: creates and activates the next
/// ontology version.
pub(crate) async fn create_ontology(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<CreateOntologyRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<OntologyResponse>), ApiError> {
    let graph_id = graph_id(path)?;
    let Json(request) = body?;
    let ontology = state
        .ontology
        .create_version(graph_id, &request.into())
        .await?;
    Ok((StatusCode::CREATED, Json(OntologyResponse::from(&ontology))))
}

fn multipart_error(err: &MultipartError, max_bytes: u64) -> ApiError {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
            format!("The upload exceeds the maximum size of {max_bytes} bytes"),
        )
    } else {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "INVALID_MULTIPART",
            "The multipart/form-data body could not be read",
        )
    }
}

/// `POST /api/v1/graphs/{graph_id}/data-sources`: streams the `file` part of
/// a `multipart/form-data` body into source storage.
pub(crate) async fn upload_data_source(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, HeaderMap, Json<DataSourceResponse>), ApiError> {
    let graph_id = graph_id(path)?;
    let mut multipart = multipart.map_err(|_| {
        ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "UNSUPPORTED_MEDIA_TYPE",
            "Expected `Content-Type: multipart/form-data` with a `file` part",
        )
    })?;
    let max_bytes = state.ingestion.config().max_upload_bytes;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|err| multipart_error(&err, max_bytes))?
    {
        if field.name() != Some(FILE_FIELD) {
            continue;
        }
        let filename = field.file_name().map(str::to_owned).ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "MISSING_FILENAME",
                "The `file` part needs a filename ending in .csv or .json",
            )
        })?;
        let mut upload = state.ingestion.begin_upload(graph_id, &filename).await?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|err| multipart_error(&err, max_bytes))?
        {
            upload.write_chunk(&chunk).await?;
        }
        let data_source = upload.finish().await?;
        let headers = location(&format!(
            "/api/v1/graphs/{graph_id}/data-sources/{}",
            data_source.id()
        ));
        let view = mindgraph_core::DataSourceView {
            data_source,
            profile: None,
        };
        return Ok((
            StatusCode::CREATED,
            headers,
            Json(DataSourceResponse::from(view)),
        ));
    }
    Err(ApiError::new(
        StatusCode::BAD_REQUEST,
        "MISSING_FILE",
        "The multipart body has no `file` part",
    ))
}

/// `GET /api/v1/graphs/{graph_id}/data-sources/{data_source_id}`
pub(crate) async fn get_data_source(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<Json<DataSourceResponse>, ApiError> {
    let (graph_id, data_source_id) = data_source_path(path)?;
    let view = state
        .ingestion
        .get_data_source(graph_id, data_source_id)
        .await?;
    Ok(Json(DataSourceResponse::from(view)))
}

/// `POST /api/v1/graphs/{graph_id}/data-sources/{data_source_id}/analyze`
pub(crate) async fn analyze_data_source(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<(StatusCode, HeaderMap, Json<JobAcceptedResponse>), ApiError> {
    let (graph_id, data_source_id) = data_source_path(path)?;
    let job = state
        .ingestion
        .start_analysis(graph_id, data_source_id)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        location(&format!("/api/v1/jobs/{}", job.id)),
        Json(JobAcceptedResponse::from(&job)),
    ))
}

/// `POST /api/v1/graphs/{graph_id}/data-sources/{data_source_id}/ingestions`
/// with a required `Idempotency-Key` header.
pub(crate) async fn start_ingestion(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
    headers: HeaderMap,
    body: Result<Json<IngestionRequest>, JsonRejection>,
) -> Result<(StatusCode, HeaderMap, Json<IngestionAcceptedResponse>), ApiError> {
    let (graph_id, data_source_id) = data_source_path(path)?;
    let key = headers
        .get("idempotency-key")
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "MISSING_IDEMPOTENCY_KEY",
                "The `Idempotency-Key` header is required",
            )
        })?
        .to_str()
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "INVALID_IDEMPOTENCY_KEY",
                "The `Idempotency-Key` header must be visible ASCII",
            )
        })?
        .to_owned();
    let Json(request) = body?;
    let job = state
        .ingestion
        .start_ingestion(graph_id, data_source_id, &key, request.mapping)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        location(&format!("/api/v1/jobs/{}", job.id)),
        Json(IngestionAcceptedResponse::from(&job)),
    ))
}

/// `GET /api/v1/jobs/{job_id}`
pub(crate) async fn get_job(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<JobResponse>, ApiError> {
    let job = state.ingestion.get_job(job_path(path)?).await?;
    Ok(Json(JobResponse::from(&job)))
}

/// The SSE event name for a job state.
const fn event_name(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued | JobStatus::Running => "progress",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Cancelled => "cancelled",
    }
}

fn job_event(job: &IngestionJob) -> Event {
    Event::default()
        .event(event_name(job.status))
        .json_data(JobResponse::from(job))
        .unwrap_or_else(|_| Event::default().event("error").data("{}"))
}

/// Polls persisted job state and yields an event whenever it changes.
struct JobWatch {
    service: IngestionService,
    id: IngestionJobId,
    next: Option<IngestionJob>,
    last: Option<(JobStatus, u64)>,
    finished: bool,
}

impl JobWatch {
    async fn next_event(mut self) -> Option<(Result<Event, Infallible>, Self)> {
        if self.finished {
            return None;
        }
        loop {
            let job = match self.next.take() {
                Some(job) => job,
                None => {
                    tokio::time::sleep(EVENT_POLL_INTERVAL).await;
                    match self.service.get_job(self.id).await {
                        Ok(job) => job,
                        Err(_) => {
                            self.finished = true;
                            let event = Event::default().event("error").data(
                                r#"{"code":"JOB_UNAVAILABLE","message":"Job state could not be read"}"#,
                            );
                            return Some((Ok(event), self));
                        }
                    }
                }
            };
            let state = (job.status, job.progress.to_bits());
            if self.last == Some(state) {
                continue;
            }
            self.last = Some(state);
            self.finished = job.status.is_terminal();
            return Some((Ok(job_event(&job)), self));
        }
    }
}

/// `GET /api/v1/jobs/{job_id}/events`: Server-Sent Events. Emits `progress`
/// while the job is queued or running, then one terminal `completed`,
/// `failed` or `cancelled` event and closes. Every event carries the job.
pub(crate) async fn job_events(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let id = job_path(path)?;
    // Unknown jobs are a normal 404, not an empty stream.
    let first = state.ingestion.get_job(id).await?;
    let watch = JobWatch {
        service: state.ingestion.as_ref().clone(),
        id,
        next: Some(first),
        last: None,
        finished: false,
    };
    let events = stream::unfold(watch, JobWatch::next_event);
    Ok(Sse::new(events).keep_alive(KeepAlive::new().interval(EVENT_KEEP_ALIVE)))
}
