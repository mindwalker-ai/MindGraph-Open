//! Runtime configuration from environment variables (ADR-013).

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use mindgraph_core::{
    DEFAULT_BATCH_SIZE, DEFAULT_JOB_CONCURRENCY, DEFAULT_MAX_UPLOAD_BYTES, GraphBuilderConfig,
    IngestionConfig, MAX_BATCH_SIZE, MAX_REPAIR_ATTEMPTS,
};
use mindgraph_ingestion::Limits;

const BIND_ADDR_VAR: &str = "MINDGRAPH_CORE_BIND_ADDR";
const DATABASE_URL_VAR: &str = "MINDGRAPH_DATABASE_URL";
const MAX_CONNECTIONS_VAR: &str = "MINDGRAPH_DATABASE_MAX_CONNECTIONS";
const HUGEGRAPH_URL_VAR: &str = "MINDGRAPH_HUGEGRAPH_URL";
const HUGEGRAPH_GRAPHSPACE_VAR: &str = "MINDGRAPH_HUGEGRAPH_GRAPHSPACE";
const HUGEGRAPH_GRAPH_VAR: &str = "MINDGRAPH_HUGEGRAPH_GRAPH";
const DATA_DIR_VAR: &str = "MINDGRAPH_DATA_DIR";
const MAX_UPLOAD_BYTES_VAR: &str = "MINDGRAPH_MAX_UPLOAD_BYTES";
const MAX_RECORDS_VAR: &str = "MINDGRAPH_MAX_INGESTION_RECORDS";
const BATCH_SIZE_VAR: &str = "MINDGRAPH_INGESTION_BATCH_SIZE";
const JOB_CONCURRENCY_VAR: &str = "MINDGRAPH_JOB_CONCURRENCY";
const AI_URL_VAR: &str = "MINDGRAPH_AI_URL";
const AI_TIMEOUT_VAR: &str = "MINDGRAPH_AI_TIMEOUT_SECONDS";
const AI_REPAIR_VAR: &str = "MINDGRAPH_AI_MAX_REPAIR_ATTEMPTS";

/// Local-first default: only reachable from this machine. Docker Compose
/// overrides it with `0.0.0.0:8080` inside the container network.
const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";
const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const MAX_MAX_CONNECTIONS: u32 = 100;
/// Source files go to `<data dir>/sources`. Docker Compose uses `/data`.
const DEFAULT_DATA_DIR: &str = "./data";
const MAX_MAX_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const DEFAULT_MAX_RECORDS: u64 = 100_000;
const MAX_MAX_RECORDS: u64 = 10_000_000;
const MAX_JOB_CONCURRENCY: usize = 16;
const DEFAULT_AI_TIMEOUT_SECONDS: u64 = 300;
const MAX_AI_TIMEOUT_SECONDS: u64 = 900;

/// Configuration of the `mindgraph-core` process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The socket address the HTTP server binds to.
    pub bind_addr: SocketAddr,
    /// The metadata database.
    pub database: DatabaseConfig,
    /// The graph database.
    pub hugegraph: HugeGraphSettings,
    /// Where uploaded source files are stored.
    pub data_dir: PathBuf,
    /// Upload, record, batch and concurrency bounds.
    pub ingestion: IngestionConfig,
    /// The internal AI service (optional: AI is disabled without it).
    pub ai: AiSettings,
}

/// Settings of the internal AI service. Its URL is internal infrastructure,
/// so `Debug` does not print it.
#[derive(Clone, PartialEq, Eq)]
pub struct AiSettings {
    url: Option<String>,
    /// Time allowed for one suggestion call.
    pub timeout: Duration,
    /// Repair attempts after Rust rejects a suggestion (`0..=2`).
    pub max_repair_attempts: u32,
}

impl AiSettings {
    /// The base URL of the AI service, or `None` when AI is disabled.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

impl fmt::Debug for AiSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiSettings")
            .field("enabled", &self.url.is_some())
            .field("timeout", &self.timeout)
            .field("max_repair_attempts", &self.max_repair_attempts)
            .finish()
    }
}

/// HugeGraph settings. `Debug` never prints the URL, which may contain
/// credentials.
#[derive(Clone, PartialEq, Eq)]
pub struct HugeGraphSettings {
    url: String,
    /// The graphspace, or the adapter default.
    pub graphspace: Option<String>,
    /// The graph name, or the adapter default.
    pub graph: Option<String>,
}

impl HugeGraphSettings {
    /// The REST base URL. Do not log it.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl fmt::Debug for HugeGraphSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HugeGraphSettings")
            .field("url", &"<redacted>")
            .field("graphspace", &self.graphspace)
            .field("graph", &self.graph)
            .finish()
    }
}

/// PostgreSQL settings. `Debug` never prints the URL, which may contain a
/// password.
#[derive(Clone, PartialEq, Eq)]
pub struct DatabaseConfig {
    url: String,
    /// Maximum pooled connections.
    pub max_connections: u32,
}

impl DatabaseConfig {
    /// The connection URL. Do not log it.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("url", &"<redacted>")
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

/// An invalid or missing configuration value. Messages never include the
/// database URL.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// `MINDGRAPH_CORE_BIND_ADDR` is not a valid `host:port` socket address.
    #[error("{BIND_ADDR_VAR} must be a socket address like 127.0.0.1:8080, got `{0}`")]
    InvalidBindAddr(String),
    /// `MINDGRAPH_DATABASE_URL` is not set.
    #[error(
        "{DATABASE_URL_VAR} is required, e.g. postgres://user:password@localhost:5432/mindgraph"
    )]
    MissingDatabaseUrl,
    /// `MINDGRAPH_DATABASE_MAX_CONNECTIONS` is not a number in range.
    #[error("{MAX_CONNECTIONS_VAR} must be a number from 1 to {MAX_MAX_CONNECTIONS}, got `{0}`")]
    InvalidMaxConnections(String),
    /// `MINDGRAPH_HUGEGRAPH_URL` is not set.
    #[error("{HUGEGRAPH_URL_VAR} is required, e.g. http://localhost:8081")]
    MissingHugeGraphUrl,
    /// `MINDGRAPH_HUGEGRAPH_URL` contains `user:password@`. HugeGraph
    /// authentication is not supported in v0.1, and URLs end up in client
    /// error messages, so credentials are refused instead of leaked.
    #[error("{HUGEGRAPH_URL_VAR} must not contain credentials")]
    HugeGraphCredentials,
    /// A numeric setting is not a number in its range.
    #[error("{name} must be a number from {min} to {max}, got `{value}`")]
    OutOfRange {
        /// The variable.
        name: &'static str,
        /// The smallest accepted value.
        min: u64,
        /// The largest accepted value.
        max: u64,
        /// The rejected value.
        value: String,
    },
}

fn number(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: u64,
    max: u64,
) -> Result<u64, ConfigError> {
    match lookup(name) {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse()
            .ok()
            .filter(|value| (1..=max).contains(value))
            .ok_or(ConfigError::OutOfRange {
                name,
                min: 1,
                max,
                value: raw,
            }),
    }
}

fn optional_text(lookup: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    lookup(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// `usize` from a bounded `u64`; every bound used here fits.
fn small(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

impl Config {
    /// Reads the configuration from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a variable is missing or invalid.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let raw_bind = lookup(BIND_ADDR_VAR).unwrap_or_else(|| DEFAULT_BIND_ADDR.to_owned());
        let bind_addr = raw_bind
            .trim()
            .parse()
            .map_err(|_| ConfigError::InvalidBindAddr(raw_bind.clone()))?;

        let url = lookup(DATABASE_URL_VAR)
            .map(|url| url.trim().to_owned())
            .filter(|url| !url.is_empty())
            .ok_or(ConfigError::MissingDatabaseUrl)?;
        let max_connections = match lookup(MAX_CONNECTIONS_VAR) {
            None => DEFAULT_MAX_CONNECTIONS,
            Some(raw) => raw
                .trim()
                .parse()
                .ok()
                .filter(|value| (1..=MAX_MAX_CONNECTIONS).contains(value))
                .ok_or(ConfigError::InvalidMaxConnections(raw))?,
        };

        let hugegraph_url =
            optional_text(&lookup, HUGEGRAPH_URL_VAR).ok_or(ConfigError::MissingHugeGraphUrl)?;
        let authority = hugegraph_url
            .split_once("://")
            .map_or(hugegraph_url.as_str(), |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or_default();
        if authority.contains('@') {
            return Err(ConfigError::HugeGraphCredentials);
        }
        let hugegraph = HugeGraphSettings {
            url: hugegraph_url,
            graphspace: optional_text(&lookup, HUGEGRAPH_GRAPHSPACE_VAR),
            graph: optional_text(&lookup, HUGEGRAPH_GRAPH_VAR),
        };
        let data_dir = optional_text(&lookup, DATA_DIR_VAR)
            .unwrap_or_else(|| DEFAULT_DATA_DIR.to_owned())
            .into();
        let defaults = IngestionConfig::default();
        let ingestion = IngestionConfig {
            max_upload_bytes: number(
                &lookup,
                MAX_UPLOAD_BYTES_VAR,
                DEFAULT_MAX_UPLOAD_BYTES,
                MAX_MAX_UPLOAD_BYTES,
            )?,
            limits: Limits {
                max_records: number(
                    &lookup,
                    MAX_RECORDS_VAR,
                    DEFAULT_MAX_RECORDS,
                    MAX_MAX_RECORDS,
                )?,
                ..defaults.limits
            },
            batch_size: small(number(
                &lookup,
                BATCH_SIZE_VAR,
                DEFAULT_BATCH_SIZE as u64,
                MAX_BATCH_SIZE as u64,
            )?),
            job_concurrency: small(number(
                &lookup,
                JOB_CONCURRENCY_VAR,
                DEFAULT_JOB_CONCURRENCY as u64,
                MAX_JOB_CONCURRENCY as u64,
            )?),
            ..defaults
        };
        let max_repair_attempts = match lookup(AI_REPAIR_VAR) {
            None => GraphBuilderConfig::default().max_repair_attempts,
            Some(raw) => raw
                .trim()
                .parse()
                .ok()
                .filter(|value| *value <= MAX_REPAIR_ATTEMPTS)
                .ok_or(ConfigError::OutOfRange {
                    name: AI_REPAIR_VAR,
                    min: 0,
                    max: u64::from(MAX_REPAIR_ATTEMPTS),
                    value: raw,
                })?,
        };
        let ai = AiSettings {
            url: optional_text(&lookup, AI_URL_VAR),
            timeout: Duration::from_secs(number(
                &lookup,
                AI_TIMEOUT_VAR,
                DEFAULT_AI_TIMEOUT_SECONDS,
                MAX_AI_TIMEOUT_SECONDS,
            )?),
            max_repair_attempts,
        };

        Ok(Self {
            bind_addr,
            database: DatabaseConfig {
                url,
                max_connections,
            },
            hugegraph,
            data_dir,
            ingestion,
            ai,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "postgres://mindgraph:s3cret@postgres:5432/mindgraph";
    const HUGEGRAPH: &str = "http://hugegraph:8080";

    /// Looks up `vars`, plus the two required URLs unless overridden.
    fn config(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        Config::from_lookup(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
                .or_else(|| (name == HUGEGRAPH_URL_VAR).then(|| HUGEGRAPH.to_owned()))
        })
    }

    #[test]
    fn defaults_to_localhost_and_ten_connections() {
        let config = config(&[(DATABASE_URL_VAR, URL)]).unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1:8080".parse().unwrap());
        assert_eq!(config.database.url(), URL);
        assert_eq!(config.database.max_connections, 10);
    }

    #[test]
    fn reads_every_variable() {
        let config = config(&[
            (BIND_ADDR_VAR, "0.0.0.0:9000"),
            (DATABASE_URL_VAR, URL),
            (MAX_CONNECTIONS_VAR, "25"),
        ])
        .unwrap();
        assert_eq!(config.bind_addr, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(config.database.max_connections, 25);
    }

    #[test]
    fn ingestion_settings_have_documented_defaults() {
        let config = config(&[(DATABASE_URL_VAR, URL)]).unwrap();
        assert_eq!(config.hugegraph.url(), HUGEGRAPH);
        assert_eq!(config.hugegraph.graphspace, None);
        assert_eq!(config.data_dir, PathBuf::from("./data"));
        assert_eq!(config.ingestion.max_upload_bytes, 50 * 1024 * 1024);
        assert_eq!(config.ingestion.limits.max_records, 100_000);
        assert_eq!(config.ingestion.batch_size, 500);
        assert_eq!(config.ingestion.job_concurrency, 2);
    }

    #[test]
    fn ingestion_settings_are_read_and_bounded() {
        let parsed = config(&[
            (DATABASE_URL_VAR, URL),
            (HUGEGRAPH_GRAPHSPACE_VAR, "SPACE"),
            (HUGEGRAPH_GRAPH_VAR, "fraud"),
            (DATA_DIR_VAR, "/data"),
            (MAX_UPLOAD_BYTES_VAR, "1024"),
            (MAX_RECORDS_VAR, "10"),
            (BATCH_SIZE_VAR, "50"),
            (JOB_CONCURRENCY_VAR, "4"),
        ])
        .unwrap();
        assert_eq!(parsed.hugegraph.graphspace.as_deref(), Some("SPACE"));
        assert_eq!(parsed.hugegraph.graph.as_deref(), Some("fraud"));
        assert_eq!(parsed.data_dir, PathBuf::from("/data"));
        assert_eq!(parsed.ingestion.max_upload_bytes, 1024);
        assert_eq!(parsed.ingestion.limits.max_records, 10);
        assert_eq!(parsed.ingestion.batch_size, 50);
        assert_eq!(parsed.ingestion.job_concurrency, 4);
        for (name, bad) in [
            (MAX_UPLOAD_BYTES_VAR, "0"),
            (BATCH_SIZE_VAR, "5001"),
            (JOB_CONCURRENCY_VAR, "17"),
            (MAX_RECORDS_VAR, "many"),
        ] {
            assert!(
                matches!(
                    config(&[(DATABASE_URL_VAR, URL), (name, bad)]),
                    Err(ConfigError::OutOfRange { .. })
                ),
                "{name}={bad}"
            );
        }
    }

    #[test]
    fn ai_is_optional_and_bounded() {
        let defaults = config(&[(DATABASE_URL_VAR, URL)]).unwrap();
        assert_eq!(
            defaults.ai.url(),
            None,
            "AI is disabled without MINDGRAPH_AI_URL"
        );
        assert_eq!(defaults.ai.timeout, Duration::from_secs(300));
        assert_eq!(defaults.ai.max_repair_attempts, 1);
        let configured = config(&[
            (DATABASE_URL_VAR, URL),
            (AI_URL_VAR, "http://ai:8000"),
            (AI_TIMEOUT_VAR, "60"),
            (AI_REPAIR_VAR, "0"),
        ])
        .unwrap();
        assert_eq!(configured.ai.url(), Some("http://ai:8000"));
        assert_eq!(configured.ai.timeout, Duration::from_secs(60));
        assert_eq!(configured.ai.max_repair_attempts, 0);
        assert!(!format!("{:?}", configured.ai).contains("ai:8000"));
        for (name, bad) in [
            (AI_REPAIR_VAR, "3"),
            (AI_TIMEOUT_VAR, "0"),
            (AI_TIMEOUT_VAR, "901"),
        ] {
            assert!(
                matches!(
                    config(&[(DATABASE_URL_VAR, URL), (name, bad)]),
                    Err(ConfigError::OutOfRange { .. })
                ),
                "{name}={bad}"
            );
        }
    }

    #[test]
    fn hugegraph_url_is_required_and_has_no_credentials() {
        let missing =
            Config::from_lookup(|name| (name == DATABASE_URL_VAR).then(|| URL.to_owned()));
        assert!(matches!(missing, Err(ConfigError::MissingHugeGraphUrl)));
        let with_credentials = config(&[
            (DATABASE_URL_VAR, URL),
            (HUGEGRAPH_URL_VAR, "http://admin:hg-s3cret@hugegraph:8080/"),
        ]);
        let err = with_credentials.unwrap_err();
        assert!(matches!(err, ConfigError::HugeGraphCredentials));
        assert!(!err.to_string().contains("hg-s3cret"));
    }

    #[test]
    fn database_url_is_required() {
        assert!(matches!(config(&[]), Err(ConfigError::MissingDatabaseUrl)));
        assert!(matches!(
            config(&[(DATABASE_URL_VAR, "  ")]),
            Err(ConfigError::MissingDatabaseUrl)
        ));
    }

    #[test]
    fn invalid_values_are_rejected() {
        assert!(matches!(
            config(&[(DATABASE_URL_VAR, URL), (BIND_ADDR_VAR, "localhost")]),
            Err(ConfigError::InvalidBindAddr(_))
        ));
        for bad in ["0", "101", "ten"] {
            assert!(matches!(
                config(&[(DATABASE_URL_VAR, URL), (MAX_CONNECTIONS_VAR, bad)]),
                Err(ConfigError::InvalidMaxConnections(_))
            ));
        }
    }

    #[test]
    fn debug_output_never_contains_the_database_url() {
        let config = config(&[(DATABASE_URL_VAR, URL)]).unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("s3cret"), "{debug}");
        assert!(!debug.contains("hugegraph:8080"), "{debug}");
        assert!(debug.contains("<redacted>"));
    }
}
