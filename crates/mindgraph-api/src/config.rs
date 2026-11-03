//! Runtime configuration from environment variables (ADR-013).

use std::net::SocketAddr;

/// Environment variable holding the address the server listens on.
const BIND_ADDR_VAR: &str = "MINDGRAPH_CORE_BIND_ADDR";

/// Local-first default: only reachable from this machine. Docker Compose
/// overrides it with `0.0.0.0:8080` inside the container network.
const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";

/// Configuration of the `mindgraph-core` process.
///
/// M0 only needs the listen address. Database, HugeGraph and AI service
/// settings (`MINDGRAPH_DATABASE_URL`, `MINDGRAPH_HUGEGRAPH_URL`,
/// `MINDGRAPH_AI_URL`) are added when the milestones that use them land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The socket address the HTTP server binds to.
    pub bind_addr: SocketAddr,
}

/// An invalid configuration value.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// `MINDGRAPH_CORE_BIND_ADDR` is not a valid `host:port` socket address.
    #[error("{BIND_ADDR_VAR} must be a socket address like 127.0.0.1:8080, got `{0}`")]
    InvalidBindAddr(String),
}

impl Config {
    /// Reads the configuration from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a variable is set to an invalid value.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let raw = lookup(BIND_ADDR_VAR).unwrap_or_else(|| DEFAULT_BIND_ADDR.to_owned());
        let bind_addr = raw
            .trim()
            .parse()
            .map_err(|_| ConfigError::InvalidBindAddr(raw.clone()))?;
        Ok(Self { bind_addr })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_localhost_only() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1:8080".parse().unwrap());
    }

    #[test]
    fn reads_bind_address_from_environment() {
        let config =
            Config::from_lookup(|name| (name == BIND_ADDR_VAR).then(|| "0.0.0.0:9000".to_owned()))
                .unwrap();
        assert_eq!(config.bind_addr, "0.0.0.0:9000".parse().unwrap());
    }

    #[test]
    fn rejects_invalid_bind_address() {
        let err = Config::from_lookup(|_| Some("localhost".to_owned())).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidBindAddr(value) if value == "localhost"));
    }
}
