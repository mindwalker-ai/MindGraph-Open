//! Runtime configuration from environment variables (ADR-013).

use std::fmt;
use std::net::SocketAddr;

const BIND_ADDR_VAR: &str = "MINDGRAPH_CORE_BIND_ADDR";
const DATABASE_URL_VAR: &str = "MINDGRAPH_DATABASE_URL";
const MAX_CONNECTIONS_VAR: &str = "MINDGRAPH_DATABASE_MAX_CONNECTIONS";

/// Local-first default: only reachable from this machine. Docker Compose
/// overrides it with `0.0.0.0:8080` inside the container network.
const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";
const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const MAX_MAX_CONNECTIONS: u32 = 100;

/// Configuration of the `mindgraph-core` process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The socket address the HTTP server binds to.
    pub bind_addr: SocketAddr,
    /// The metadata database.
    pub database: DatabaseConfig,
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

        Ok(Self {
            bind_addr,
            database: DatabaseConfig {
                url,
                max_connections,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "postgres://mindgraph:s3cret@postgres:5432/mindgraph";

    fn config(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        Config::from_lookup(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
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
        assert!(debug.contains("<redacted>"));
    }
}
