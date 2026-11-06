//! The application error type shared by all use cases.

use mindgraph_domain::{DataSourceId, DomainError, EntityId, GraphId, IngestionJobId};
use mindgraph_graph::GraphStoreError;
use mindgraph_ingestion::IngestionError;

use crate::MetadataStoreError;

/// A boxed underlying error, kept for logs.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// An application-level error, independent of any transport.
#[derive(Debug, thiserror::Error)]
pub enum ApplicationError {
    /// The input breaks a domain rule, for example an empty graph name.
    #[error(transparent)]
    Validation(#[from] DomainError),

    /// The request is malformed, for example a limit out of range.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// The graph does not exist in the local workspace.
    #[error("graph {0} was not found")]
    GraphNotFound(GraphId),

    /// The data source does not exist in the requested graph.
    #[error("data source {0} was not found")]
    DataSourceNotFound(DataSourceId),

    /// The job does not exist.
    #[error("job {0} was not found")]
    JobNotFound(IngestionJobId),

    /// The entity does not exist in the requested graph (an entity of
    /// another graph is missing too).
    #[error("entity {0} was not found")]
    EntityNotFound(EntityId),

    /// The write conflicts with existing data or the current state.
    #[error("conflict: {0}")]
    Conflict(String),

    /// The uploaded file type is not supported.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    /// The upload exceeds the configured limit.
    #[error("the upload exceeds the maximum size of {max_bytes} bytes")]
    PayloadTooLarge {
        /// The configured limit.
        max_bytes: u64,
    },

    /// The request is well-formed but cannot be processed, for example an
    /// invalid mapping. The message never contains dataset values.
    #[error("{message}")]
    Unprocessable {
        /// A stable code such as `INVALID_MAPPING`.
        code: &'static str,
        /// A client-safe message.
        message: String,
    },

    /// The AI Graph Builder is disabled (no provider configured).
    #[error("the AI Graph Builder is not configured")]
    AiNotConfigured,

    /// The AI service cannot be reached.
    #[error("the AI service is unavailable")]
    AiUnavailable,

    /// A GraphRAG question failed (invalid input, plan or AI failure).
    #[error(transparent)]
    GraphRag(crate::GraphRagError),

    /// An entity resolution request failed (invalid rules, unknown scan or
    /// candidate, decision conflict, scan timeout).
    #[error(transparent)]
    Resolution(crate::ResolutionError),

    /// A required backend (metadata database, graph store, source storage)
    /// is unavailable, or the job queue is full.
    #[error("a required backend is unavailable")]
    Unavailable(#[source] BoxError),

    /// An unexpected failure. The details are for logs, not for clients.
    #[error("internal error: {0}")]
    Internal(#[source] BoxError),
}

impl ApplicationError {
    /// Wraps an unexpected error.
    pub fn internal(error: impl Into<BoxError>) -> Self {
        Self::Internal(error.into())
    }

    /// Wraps a backend-unavailable error.
    pub fn unavailable(error: impl Into<BoxError>) -> Self {
        Self::Unavailable(error.into())
    }
}

impl From<MetadataStoreError> for ApplicationError {
    fn from(error: MetadataStoreError) -> Self {
        match error {
            // Store messages may name constraints; clients get a stable text.
            MetadataStoreError::Conflict(message) => {
                tracing::debug!(error = %message, "metadata store conflict");
                Self::Conflict("the request conflicts with existing data".to_owned())
            }
            MetadataStoreError::BackendUnavailable { .. } => Self::unavailable(error),
            // The services validate their inputs, so anything else is a bug
            // or corrupt data rather than a client mistake.
            other => Self::internal(other),
        }
    }
}

impl From<IngestionError> for ApplicationError {
    fn from(error: IngestionError) -> Self {
        match error {
            IngestionError::UnsupportedFormat(message) => Self::UnsupportedFormat(message),
            IngestionError::FileTooLarge { max_bytes } => Self::PayloadTooLarge { max_bytes },
            IngestionError::SourceStore { .. }
            | IngestionError::GraphStore(GraphStoreError::BackendUnavailable { .. }) => {
                Self::unavailable(error)
            }
            IngestionError::GraphStore(_) => Self::internal(error),
            other => Self::Unprocessable {
                code: other.code(),
                message: other.public_message(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_errors_map_to_application_categories() {
        let unavailable = ApplicationError::from(MetadataStoreError::BackendUnavailable {
            message: "down".into(),
            source: None,
        });
        assert!(matches!(unavailable, ApplicationError::Unavailable(_)));
        assert!(matches!(
            ApplicationError::from(MetadataStoreError::Conflict("dup".into())),
            ApplicationError::Conflict(_)
        ));
        assert!(matches!(
            ApplicationError::from(MetadataStoreError::CorruptData("bad".into())),
            ApplicationError::Internal(_)
        ));
    }

    #[test]
    fn ingestion_errors_map_to_application_categories() {
        assert!(matches!(
            ApplicationError::from(IngestionError::FileTooLarge { max_bytes: 9 }),
            ApplicationError::PayloadTooLarge { max_bytes: 9 }
        ));
        assert!(matches!(
            ApplicationError::from(IngestionError::UnsupportedFormat("x".into())),
            ApplicationError::UnsupportedFormat(_)
        ));
        assert!(matches!(
            ApplicationError::from(IngestionError::InvalidMapping("x".into())),
            ApplicationError::Unprocessable {
                code: "INVALID_MAPPING",
                ..
            }
        ));
        assert!(matches!(
            ApplicationError::from(IngestionError::SourceStore {
                message: "x".into(),
                source: None
            }),
            ApplicationError::Unavailable(_)
        ));
        assert!(matches!(
            ApplicationError::from(IngestionError::GraphStore(
                GraphStoreError::BackendUnavailable {
                    message: "x".into(),
                    source: None
                }
            )),
            ApplicationError::Unavailable(_)
        ));
    }
}
