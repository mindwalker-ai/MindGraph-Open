//! The ingestion error type.
//!
//! Messages identify records (1-based) and field names but never include
//! field values, so they are safe to log and return to clients.

use mindgraph_domain::PropertyType;
use mindgraph_graph::GraphStoreError;

/// An error from parsing, profiling, mapping or ingesting a dataset.
#[derive(Debug, thiserror::Error)]
pub enum IngestionError {
    /// The file type or a value shape is not supported (e.g. nested objects).
    #[error("unsupported input: {0}")]
    UnsupportedFormat(String),

    /// The upload exceeds the configured size limit.
    #[error("the file exceeds the maximum size of {max_bytes} bytes")]
    FileTooLarge {
        /// The configured limit.
        max_bytes: u64,
    },

    /// The file is not valid CSV / JSON of the supported shape.
    #[error("malformed dataset: {0}")]
    MalformedDataset(String),

    /// The header or field set is unusable (empty or duplicate headers).
    #[error("invalid schema: {0}")]
    InvalidSchema(String),

    /// The mapping does not fit the ontology or the dataset.
    #[error("invalid mapping: {0}")]
    InvalidMapping(String),

    /// A required value is blank or null.
    #[error("record {record}: missing value for field `{field}`")]
    MissingField {
        /// The 1-based record number.
        record: u64,
        /// The source field.
        field: String,
    },

    /// A value cannot be converted to its target property type.
    #[error("record {record}: field `{field}` cannot be converted to {expected}")]
    TypeConversion {
        /// The 1-based record number.
        record: u64,
        /// The source field.
        field: String,
        /// The ontology property type.
        expected: PropertyType,
    },

    /// More than one existing entity matches an identity.
    #[error("record {record}: more than one existing `{entity_type}` matches its identity")]
    AmbiguousEntity {
        /// The 1-based record number.
        record: u64,
        /// The entity type.
        entity_type: String,
    },

    /// A `match_only` entity does not exist.
    #[error("record {record}: no existing `{entity_type}` matches its identity")]
    EntityNotFound {
        /// The 1-based record number.
        record: u64,
        /// The entity type.
        entity_type: String,
    },

    /// The graph store failed.
    #[error("graph store: {0}")]
    GraphStore(#[from] GraphStoreError),

    /// Source-file storage failed.
    #[error("source storage: {message}")]
    SourceStore {
        /// What failed.
        message: String,
        /// The underlying error, kept for diagnostics.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
    },

    /// A configured bound (records, fields, value size) was exceeded.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
}

impl IngestionError {
    /// A stable, machine-readable code, for example `TYPE_CONVERSION`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedFormat(_) => "UNSUPPORTED_FORMAT",
            Self::FileTooLarge { .. } => "FILE_TOO_LARGE",
            Self::MalformedDataset(_) => "MALFORMED_DATASET",
            Self::InvalidSchema(_) => "INVALID_SCHEMA",
            Self::InvalidMapping(_) => "INVALID_MAPPING",
            Self::MissingField { .. } => "MISSING_FIELD",
            Self::TypeConversion { .. } => "TYPE_CONVERSION",
            Self::AmbiguousEntity { .. } => "AMBIGUOUS_ENTITY",
            Self::EntityNotFound { .. } => "ENTITY_NOT_FOUND",
            Self::GraphStore(GraphStoreError::BackendUnavailable { .. }) => "GRAPH_UNAVAILABLE",
            Self::GraphStore(_) => "GRAPH_STORE_ERROR",
            Self::SourceStore { .. } => "SOURCE_STORAGE_ERROR",
            Self::LimitExceeded(_) => "LIMIT_EXCEEDED",
        }
    }

    /// A message that is safe to persist and show to clients.
    ///
    /// Graph-store failures are reduced to their category: backend messages
    /// are not under MindGraph's control and could echo stored data. Graph
    /// writes are not transactional across batches, so the message says that
    /// earlier batches may remain.
    #[must_use]
    pub fn public_message(&self) -> String {
        match self {
            Self::GraphStore(GraphStoreError::BackendUnavailable { .. }) => {
                "the graph backend is unavailable; batches written before the failure \
                 were not rolled back"
                    .to_owned()
            }
            Self::GraphStore(_) => "the graph backend rejected a write; batches written before \
                                    the failure were not rolled back"
                .to_owned(),
            other => other.to_string(),
        }
    }

    pub(crate) fn storage(message: impl Into<String>, source: std::io::Error) -> Self {
        Self::SourceStore {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}
