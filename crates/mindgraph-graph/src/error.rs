//! The storage-layer error type.

use mindgraph_domain::{EntityId, GraphId};

/// An error returned by a [`GraphStore`](crate::GraphStore) operation.
///
/// The variants are backend-independent categories. Adapter-specific errors
/// such as HTTP client failures are never part of the public type. When they
/// carry useful diagnostics, they are kept as the error's
/// [`source`](std::error::Error::source).
#[derive(Debug, thiserror::Error)]
pub enum GraphStoreError {
    /// An entity the operation depends on does not exist in the graph, for
    /// example the source of a relationship or the start of a path.
    ///
    /// An entity that exists only in *another* graph is reported the same
    /// way: graphs never observe each other's data.
    #[error("entity {entity_id} does not exist in graph {graph_id}")]
    EntityNotFound {
        /// The graph that was searched.
        graph_id: GraphId,
        /// The missing entity.
        entity_id: EntityId,
    },

    /// The write would overwrite an existing entity or relationship.
    #[error("conflict: {0}")]
    Conflict(String),

    /// The request is invalid regardless of the stored data, for example a
    /// batch that mixes graphs or a limit outside the allowed range.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The backend could not be reached or did not answer in time.
    #[error("graph backend unavailable: {message}")]
    BackendUnavailable {
        /// What failed.
        message: String,
        /// The underlying error, kept for diagnostics.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
    },

    /// The backend rejected the request or answered in an unexpected way.
    #[error("graph backend error: {message}")]
    BackendError {
        /// What failed.
        message: String,
        /// The underlying error, kept for diagnostics.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
    },

    /// Data read from the backend cannot be turned back into domain objects.
    #[error("corrupt graph data: {0}")]
    CorruptData(String),

    /// Domain data could not be encoded for the backend.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// The backend or its configuration does not support the operation.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl GraphStoreError {
    /// A [`GraphStoreError::BackendError`] without an underlying error.
    #[cfg(feature = "hugegraph")]
    pub(crate) fn backend(message: impl Into<String>) -> Self {
        Self::BackendError {
            message: message.into(),
            source: None,
        }
    }
}
