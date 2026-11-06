//! Runtime configuration from environment variables.
//!
//! `mindgraph-mcp` serves exactly one graph project and talks only to a
//! MindGraph Core on a loopback address. Every value is validated before the
//! MCP transport starts, so a misconfigured server fails with one clear line
//! on stderr instead of answering tool calls with surprises.

use std::net::IpAddr;
use std::time::Duration;

use reqwest::Url;
use uuid::Uuid;

/// The base URL of MindGraph Core.
pub const CORE_URL_VAR: &str = "MINDGRAPH_MCP_CORE_URL";
/// The one graph project this server exposes (a UUIDv7).
pub const GRAPH_ID_VAR: &str = "MINDGRAPH_MCP_GRAPH_ID";
/// Deadline of one Core request made by a deterministic graph tool.
pub const TIMEOUT_VAR: &str = "MINDGRAPH_MCP_TIMEOUT_SECONDS";
/// Deadline of one `ask_graph` request (GraphRAG calls a model twice).
pub const ASK_TIMEOUT_VAR: &str = "MINDGRAPH_MCP_ASK_TIMEOUT_SECONDS";
/// Largest JSON result one tool call may return.
pub const MAX_RESULT_BYTES_VAR: &str = "MINDGRAPH_MCP_MAX_RESULT_BYTES";
/// Registers the optional `ask_graph` tool.
pub const ENABLE_ASK_VAR: &str = "MINDGRAPH_MCP_ENABLE_ASK";

/// Default Core URL: the port `docker compose up` publishes on loopback.
pub const DEFAULT_CORE_URL: &str = "http://127.0.0.1:8080";
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const TIMEOUT_RANGE: (u64, u64) = (1, 300);
// Core's GraphRAG deadline defaults to 120 s; leave room for its answer.
const DEFAULT_ASK_TIMEOUT_SECONDS: u64 = 150;
const ASK_TIMEOUT_RANGE: (u64, u64) = (1, 600);
/// 256 KiB.
pub const DEFAULT_MAX_RESULT_BYTES: usize = 262_144;
const MAX_RESULT_BYTES_RANGE: (usize, usize) = (16_384, 4_194_304);

/// A configuration problem, reported before the transport starts.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{variable}: {reason}")]
pub struct ConfigError {
    /// The environment variable at fault.
    pub variable: &'static str,
    /// What is wrong with it.
    pub reason: String,
}

impl ConfigError {
    fn new(variable: &'static str, reason: impl Into<String>) -> Self {
        Self {
            variable,
            reason: reason.into(),
        }
    }
}

/// Validated settings of one `mindgraph-mcp` process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Core base URL: `http`, loopback host, no credentials, no path.
    pub core_url: Url,
    /// The configured graph project; tools never accept another one.
    pub graph_id: Uuid,
    /// Deadline of one Core request for the deterministic tools.
    pub timeout: Duration,
    /// Deadline of one `ask_graph` request.
    pub ask_timeout: Duration,
    /// Largest serialized tool result, in bytes.
    pub max_result_bytes: usize,
    /// Whether `ask_graph` is registered.
    pub enable_ask: bool,
}

impl Config {
    /// Reads the configuration from the process environment.
    ///
    /// # Errors
    ///
    /// The first invalid or missing variable.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the configuration through `lookup` (the environment in
    /// production, a map in tests). Empty values count as unset.
    ///
    /// # Errors
    ///
    /// The first invalid or missing variable.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let value = |name: &str| {
            lookup(name)
                .map(|raw| raw.trim().to_owned())
                .filter(|raw| !raw.is_empty())
        };
        let core_url = core_url(value(CORE_URL_VAR).as_deref().unwrap_or(DEFAULT_CORE_URL))?;
        let graph_id = graph_id(value(GRAPH_ID_VAR).as_deref())?;
        let timeout = seconds(
            TIMEOUT_VAR,
            value(TIMEOUT_VAR).as_deref(),
            DEFAULT_TIMEOUT_SECONDS,
            TIMEOUT_RANGE,
        )?;
        let ask_timeout = seconds(
            ASK_TIMEOUT_VAR,
            value(ASK_TIMEOUT_VAR).as_deref(),
            DEFAULT_ASK_TIMEOUT_SECONDS,
            ASK_TIMEOUT_RANGE,
        )?;
        let max_result_bytes = max_result_bytes(value(MAX_RESULT_BYTES_VAR).as_deref())?;
        let enable_ask = flag(ENABLE_ASK_VAR, value(ENABLE_ASK_VAR).as_deref())?;
        Ok(Self {
            core_url,
            graph_id,
            timeout,
            ask_timeout,
            max_result_bytes,
            enable_ask,
        })
    }
}

/// Accepts only `http://<loopback>[:port]`: MindGraph Core has no
/// authentication in v0.1, so the server must not be pointed at a remote
/// host, and graph data must not leave the machine through it.
fn core_url(raw: &str) -> Result<Url, ConfigError> {
    let invalid = |reason: &str| ConfigError::new(CORE_URL_VAR, reason);
    let url = Url::parse(raw).map_err(|_| invalid("not a valid URL"))?;
    if url.scheme() != "http" {
        return Err(invalid(
            "must use http:// (mindgraph-mcp reaches a local MindGraph Core; https and remote Cores are not supported)",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("must not contain credentials"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("must not contain a query or fragment"));
    }
    if url.path() != "/" {
        return Err(invalid(
            "must be the Core base URL without a path, e.g. http://127.0.0.1:8080",
        ));
    }
    // `host_str` writes IPv6 hosts in brackets.
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if !loopback {
        return Err(invalid(
            "must point to a loopback address (127.0.0.1, ::1 or localhost); remote Cores are not supported",
        ));
    }
    Ok(url)
}

/// A non-nil UUIDv7, the MindGraph identifier format (ADR-003).
pub(crate) fn parse_uuid_v7(raw: &str) -> Option<Uuid> {
    Uuid::parse_str(raw.trim())
        .ok()
        .filter(|uuid| !uuid.is_nil() && uuid.get_version_num() == 7)
}

fn graph_id(raw: Option<&str>) -> Result<Uuid, ConfigError> {
    let raw = raw.ok_or_else(|| {
        ConfigError::new(
            GRAPH_ID_VAR,
            "is required: the id of the graph project to expose (copy it from the MCP page or GET /api/v1/graphs)",
        )
    })?;
    parse_uuid_v7(raw).ok_or_else(|| ConfigError::new(GRAPH_ID_VAR, "must be a UUIDv7 graph id"))
}

fn seconds(
    variable: &'static str,
    raw: Option<&str>,
    default: u64,
    (min, max): (u64, u64),
) -> Result<Duration, ConfigError> {
    let Some(raw) = raw else {
        return Ok(Duration::from_secs(default));
    };
    raw.parse::<u64>()
        .ok()
        .filter(|value| (min..=max).contains(value))
        .map(Duration::from_secs)
        .ok_or_else(|| {
            ConfigError::new(
                variable,
                format!("must be a whole number of seconds between {min} and {max}"),
            )
        })
}

fn max_result_bytes(raw: Option<&str>) -> Result<usize, ConfigError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_MAX_RESULT_BYTES);
    };
    let (min, max) = MAX_RESULT_BYTES_RANGE;
    raw.parse::<usize>()
        .ok()
        .filter(|value| (min..=max).contains(value))
        .ok_or_else(|| {
            ConfigError::new(
                MAX_RESULT_BYTES_VAR,
                format!("must be a number of bytes between {min} and {max}"),
            )
        })
}

fn flag(variable: &'static str, raw: Option<&str>) -> Result<bool, ConfigError> {
    match raw.map(str::to_ascii_lowercase).as_deref() {
        None | Some("false" | "0") => Ok(false),
        Some("true" | "1") => Ok(true),
        Some(_) => Err(ConfigError::new(variable, "must be true or false")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const GRAPH: &str = "019a5f6e-8b3c-7d21-9f00-1a2b3c4d5e6f";

    fn config(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Config::from_lookup(|name| vars.get(name).cloned())
    }

    fn reason(vars: &[(&str, &str)]) -> (String, &'static str) {
        let error = config(vars).unwrap_err();
        (error.reason, error.variable)
    }

    #[test]
    fn defaults_need_only_a_graph_id() {
        let config = config(&[(GRAPH_ID_VAR, GRAPH)]).unwrap();
        assert_eq!(config.core_url.as_str(), "http://127.0.0.1:8080/");
        assert_eq!(config.graph_id.to_string(), GRAPH);
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.ask_timeout, Duration::from_secs(150));
        assert_eq!(config.max_result_bytes, 262_144);
        assert!(!config.enable_ask, "ask_graph is off by default");
    }

    #[test]
    fn local_core_urls_are_accepted() {
        for url in [
            "http://127.0.0.1:18088",
            "http://localhost:8080/",
            "http://LOCALHOST:8080",
            "http://127.1.2.3:9000",
            "http://[::1]:8080",
        ] {
            let config = config(&[(GRAPH_ID_VAR, GRAPH), (CORE_URL_VAR, url)]);
            assert!(config.is_ok(), "{url}: {config:?}");
        }
    }

    #[test]
    fn remote_and_unsafe_core_urls_are_rejected() {
        for (url, expected) in [
            ("not a url", "not a valid URL"),
            ("http://10.0.0.5:8080", "loopback"),
            ("http://core:8080", "loopback"),
            ("http://mindgraph.example.com", "loopback"),
            ("http://0.0.0.0:8080", "loopback"),
            ("https://127.0.0.1:8080", "http://"),
            ("ftp://127.0.0.1", "http://"),
            ("http://admin:secret@127.0.0.1:8080", "credentials"),
            ("http://token@127.0.0.1:8080", "credentials"),
            ("http://127.0.0.1:8080/api/v1", "without a path"),
            ("http://127.0.0.1:8080/?graph=x", "query"),
        ] {
            let (reason, variable) = reason(&[(GRAPH_ID_VAR, GRAPH), (CORE_URL_VAR, url)]);
            assert_eq!(variable, CORE_URL_VAR);
            assert!(reason.contains(expected), "{url}: {reason}");
            assert!(!reason.contains("secret"), "credentials are never echoed");
        }
    }

    #[test]
    fn the_graph_id_is_required_and_must_be_a_uuid_v7() {
        let (reason, variable) = reason(&[]);
        assert_eq!(variable, GRAPH_ID_VAR);
        assert!(reason.contains("required"), "{reason}");
        assert_eq!(reason, self::reason(&[(GRAPH_ID_VAR, "  ")]).0);
        for raw in [
            "fraud-investigation",
            "00000000-0000-0000-0000-000000000000",
            "550e8400-e29b-41d4-a716-446655440000",
        ] {
            let (reason, variable) = self::reason(&[(GRAPH_ID_VAR, raw)]);
            assert_eq!(variable, GRAPH_ID_VAR);
            assert_eq!(reason, "must be a UUIDv7 graph id", "{raw}");
        }
    }

    #[test]
    fn timeouts_and_result_sizes_are_bounded() {
        let ok = config(&[
            (GRAPH_ID_VAR, GRAPH),
            (TIMEOUT_VAR, "300"),
            (ASK_TIMEOUT_VAR, "600"),
            (MAX_RESULT_BYTES_VAR, "16384"),
        ])
        .unwrap();
        assert_eq!(ok.timeout, Duration::from_secs(300));
        assert_eq!(ok.ask_timeout, Duration::from_secs(600));
        assert_eq!(ok.max_result_bytes, 16_384);
        for (variable, raw) in [
            (TIMEOUT_VAR, "0"),
            (TIMEOUT_VAR, "301"),
            (TIMEOUT_VAR, "1.5"),
            (ASK_TIMEOUT_VAR, "601"),
            (MAX_RESULT_BYTES_VAR, "1024"),
            (MAX_RESULT_BYTES_VAR, "4194305"),
            (MAX_RESULT_BYTES_VAR, "-1"),
        ] {
            let (_, at_fault) = reason(&[(GRAPH_ID_VAR, GRAPH), (variable, raw)]);
            assert_eq!(at_fault, variable, "{variable}={raw}");
        }
    }

    #[test]
    fn ask_graph_is_enabled_only_explicitly() {
        for (raw, expected) in [
            ("true", true),
            ("TRUE", true),
            ("1", true),
            ("false", false),
            ("0", false),
        ] {
            let config = config(&[(GRAPH_ID_VAR, GRAPH), (ENABLE_ASK_VAR, raw)]).unwrap();
            assert_eq!(config.enable_ask, expected, "{raw}");
        }
        let (_, variable) = reason(&[(GRAPH_ID_VAR, GRAPH), (ENABLE_ASK_VAR, "yes please")]);
        assert_eq!(variable, ENABLE_ASK_VAR);
    }
}
