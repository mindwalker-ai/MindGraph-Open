//! Configuration of the HugeGraph adapter.

use std::time::Duration;

use reqwest::Url;

use crate::GraphStoreError;

/// Connection settings for [`HugeGraphStore`](crate::HugeGraphStore).
///
/// Construction validates every value. Reading environment variables is
/// left to the application, which builds this value and passes it to
/// [`HugeGraphStore::new`](crate::HugeGraphStore::new).
///
/// ```
/// use std::time::Duration;
///
/// use mindgraph_graph::HugeGraphConfig;
///
/// let config = HugeGraphConfig::new("http://hugegraph:8080")
///     .unwrap()
///     .with_request_timeout(Duration::from_secs(10))
///     .unwrap();
/// assert_eq!(config.graphspace(), "DEFAULT");
/// assert_eq!(config.graph_name(), "hugegraph");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HugeGraphConfig {
    base_url: Url,
    graphspace: String,
    graph_name: String,
    request_timeout: Duration,
}

impl HugeGraphConfig {
    /// The graphspace of a standalone HugeGraph 1.7 server.
    pub const DEFAULT_GRAPHSPACE: &'static str = "DEFAULT";
    /// The graph created by the official HugeGraph Docker image.
    pub const DEFAULT_GRAPH_NAME: &'static str = "hugegraph";
    /// The timeout applied to every HTTP request unless overridden.
    pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

    /// Creates a configuration for the server at `base_url`, for example
    /// `http://hugegraph:8080`, with the default graphspace, graph name and
    /// timeout.
    ///
    /// # Errors
    ///
    /// [`InvalidInput`](GraphStoreError::InvalidInput) for a malformed URL,
    /// and [`Unsupported`](GraphStoreError::Unsupported) for `https`. TLS is
    /// not enabled in v0.1 because HugeGraph is an internal backend.
    pub fn new(base_url: &str) -> Result<Self, GraphStoreError> {
        let invalid = |reason: &str| {
            GraphStoreError::InvalidInput(format!("invalid HugeGraph URL `{base_url}`: {reason}"))
        };
        let url = Url::parse(base_url.trim()).map_err(|err| invalid(&err.to_string()))?;
        match url.scheme() {
            "http" => {}
            "https" => {
                return Err(GraphStoreError::Unsupported(
                    "HTTPS connections to HugeGraph are not enabled in v0.1; HugeGraph is an \
                     internal backend reached over HTTP"
                        .to_owned(),
                ));
            }
            other => return Err(invalid(&format!("unsupported scheme `{other}`"))),
        }
        if url.host_str().is_none() || url.cannot_be_a_base() {
            return Err(invalid("a host is required"));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(invalid("query strings and fragments are not allowed"));
        }
        Ok(Self {
            base_url: url,
            graphspace: Self::DEFAULT_GRAPHSPACE.to_owned(),
            graph_name: Self::DEFAULT_GRAPH_NAME.to_owned(),
            request_timeout: Self::DEFAULT_REQUEST_TIMEOUT,
        })
    }

    /// Sets the HugeGraph graphspace.
    ///
    /// # Errors
    ///
    /// [`InvalidInput`](GraphStoreError::InvalidInput) unless the name is
    /// 1–64 ASCII letters, digits, `_` or `-`.
    pub fn with_graphspace(mut self, graphspace: &str) -> Result<Self, GraphStoreError> {
        self.graphspace = check_name("graphspace", graphspace)?;
        Ok(self)
    }

    /// Sets the HugeGraph graph name.
    ///
    /// # Errors
    ///
    /// [`InvalidInput`](GraphStoreError::InvalidInput) unless the name is
    /// 1–64 ASCII letters, digits, `_` or `-`.
    pub fn with_graph_name(mut self, graph_name: &str) -> Result<Self, GraphStoreError> {
        self.graph_name = check_name("graph name", graph_name)?;
        Ok(self)
    }

    /// Sets the per-request timeout.
    ///
    /// # Errors
    ///
    /// [`InvalidInput`](GraphStoreError::InvalidInput) for a zero duration.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Result<Self, GraphStoreError> {
        if timeout.is_zero() {
            return Err(GraphStoreError::InvalidInput(
                "the HugeGraph request timeout must be greater than zero".to_owned(),
            ));
        }
        self.request_timeout = timeout;
        Ok(self)
    }

    /// The server URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    /// The graphspace.
    #[must_use]
    pub fn graphspace(&self) -> &str {
        &self.graphspace
    }

    /// The graph name.
    #[must_use]
    pub fn graph_name(&self) -> &str {
        &self.graph_name
    }

    /// The per-request timeout.
    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub(crate) const fn url(&self) -> &Url {
        &self.base_url
    }
}

fn check_name(what: &str, value: &str) -> Result<String, GraphStoreError> {
    let valid = (1..=64).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(value.to_owned())
    } else {
        Err(GraphStoreError::InvalidInput(format!(
            "HugeGraph {what} must be 1-64 ASCII letters, digits, `_` or `-`, got `{value}`"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_official_docker_image() {
        let config = HugeGraphConfig::new("http://hugegraph:8080").unwrap();
        assert_eq!(config.base_url(), "http://hugegraph:8080/");
        assert_eq!(config.graphspace(), "DEFAULT");
        assert_eq!(config.graph_name(), "hugegraph");
        assert_eq!(config.request_timeout(), Duration::from_secs(30));
    }

    #[test]
    fn overrides_are_validated() {
        let config = HugeGraphConfig::new("http://localhost:18080")
            .unwrap()
            .with_graphspace("SPACE_1")
            .unwrap()
            .with_graph_name("mindgraph-dev")
            .unwrap()
            .with_request_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(config.graphspace(), "SPACE_1");
        assert_eq!(config.graph_name(), "mindgraph-dev");

        let base = HugeGraphConfig::new("http://localhost:18080").unwrap();
        assert!(base.clone().with_graph_name("").is_err());
        assert!(base.clone().with_graph_name("a/b").is_err());
        assert!(base.clone().with_graphspace("x y").is_err());
        assert!(base.with_request_timeout(Duration::ZERO).is_err());
    }

    #[test]
    fn urls_are_validated() {
        for bad in [
            "",
            "not a url",
            "ftp://host",
            "http://host:8080/?x=1",
            "http://host#f",
        ] {
            assert!(
                matches!(
                    HugeGraphConfig::new(bad),
                    Err(GraphStoreError::InvalidInput(_))
                ),
                "{bad:?} should be rejected"
            );
        }
        assert!(matches!(
            HugeGraphConfig::new("https://hugegraph:8443"),
            Err(GraphStoreError::Unsupported(_))
        ));
    }
}
