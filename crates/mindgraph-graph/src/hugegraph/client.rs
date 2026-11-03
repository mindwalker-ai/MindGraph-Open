//! Thin HTTP layer over the HugeGraph 1.7 REST API.
//!
//! One `reqwest::Client` is built per store and reused for every request.
//! HTTP outcomes are translated into [`GraphStoreError`] categories here, so
//! no `reqwest` type escapes the adapter.

use reqwest::{RequestBuilder, Response, StatusCode, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::config::HugeGraphConfig;
use super::dto::ErrorBody;
use crate::GraphStoreError;

/// Longest HugeGraph error message copied into our errors.
const MAX_MESSAGE_CHARS: usize = 300;

/// A failed request.
#[derive(Debug)]
pub(crate) enum Failure {
    /// HugeGraph answered with a non-success status.
    Status {
        status: StatusCode,
        /// Short exception class name, e.g. `ExistedException`.
        exception: String,
        message: String,
    },
    /// Any other failure, already categorised.
    Store(GraphStoreError),
}

impl From<Failure> for GraphStoreError {
    fn from(failure: Failure) -> Self {
        let (status, exception, message) = match failure {
            Failure::Store(error) => return error,
            Failure::Status {
                status,
                exception,
                message,
            } => (status, exception, message),
        };
        let detail = if exception.is_empty() {
            message
        } else {
            format!("{exception}: {message}")
        };
        match status {
            StatusCode::CONFLICT => Self::Conflict(detail),
            StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => Self::BackendUnavailable {
                message: format!("HugeGraph returned {status}: {detail}"),
                source: None,
            },
            status if status.is_client_error() => Self::backend(format!(
                "HugeGraph rejected the request ({status}): {detail}"
            )),
            status => Self::backend(format!("HugeGraph failed ({status}): {detail}")),
        }
    }
}

impl From<GraphStoreError> for Failure {
    fn from(error: GraphStoreError) -> Self {
        Self::Store(error)
    }
}

/// HTTP access to one HugeGraph graph.
#[derive(Debug, Clone)]
pub(crate) struct Client {
    http: reqwest::Client,
    /// `{base}/graphspaces/{graphspace}/graphs/{graph}`
    graph_root: Url,
}

impl Client {
    pub(crate) fn new(config: &HugeGraphConfig) -> Result<Self, GraphStoreError> {
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout())
            .build()
            .map_err(|err| GraphStoreError::BackendError {
                message: "cannot build the HugeGraph HTTP client".to_owned(),
                source: Some(Box::new(err)),
            })?;
        let mut graph_root = config.url().clone();
        graph_root
            .path_segments_mut()
            .map_err(|()| {
                GraphStoreError::InvalidInput("the HugeGraph URL cannot have a path".to_owned())
            })?
            .pop_if_empty()
            .extend([
                "graphspaces",
                config.graphspace(),
                "graphs",
                config.graph_name(),
            ]);
        Ok(Self { http, graph_root })
    }

    /// `graph_root` + `segments`, each segment percent-encoded.
    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.graph_root.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.extend(segments);
        }
        url
    }

    /// `GET` returning JSON. Every non-success status, including 404, is
    /// an error: missing elements are detected through endpoints that
    /// return empty lists instead.
    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<T, Failure> {
        let response = send(self.http.get(self.url(segments)).query(query)).await?;
        json(check(response).await?).await
    }

    /// `POST` a JSON body and decode the JSON response.
    pub(crate) async fn post<B, T>(
        &self,
        segments: &[&str],
        query: &[(&str, String)],
        body: &B,
    ) -> Result<T, Failure>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        let response = send(self.http.post(self.url(segments)).query(query).json(body)).await?;
        json(check(response).await?).await
    }

    /// `DELETE` expecting an empty success response.
    pub(crate) async fn delete(
        &self,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<(), Failure> {
        let response = send(self.http.delete(self.url(segments)).query(query)).await?;
        check(response).await.map(drop)
    }
}

async fn send(request: RequestBuilder) -> Result<Response, Failure> {
    request.send().await.map_err(|err| {
        let error = if err.is_timeout() {
            GraphStoreError::BackendUnavailable {
                message: "HugeGraph did not answer in time".to_owned(),
                source: Some(Box::new(err)),
            }
        } else if err.is_connect() {
            GraphStoreError::BackendUnavailable {
                message: "cannot connect to HugeGraph".to_owned(),
                source: Some(Box::new(err)),
            }
        } else {
            GraphStoreError::BackendError {
                message: "the HugeGraph request failed".to_owned(),
                source: Some(Box::new(err)),
            }
        };
        Failure::Store(error)
    })
}

async fn check(response: Response) -> Result<Response, Failure> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    // Only HugeGraph's structured message is kept, never a raw body.
    let body: ErrorBody = response
        .text()
        .await
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    Err(Failure::Status {
        status,
        // "class org.apache.hugegraph.exception.ExistedException" -> "ExistedException"
        exception: body
            .exception
            .rsplit(['.', ' '])
            .next()
            .unwrap_or_default()
            .to_owned(),
        message: body.message.chars().take(MAX_MESSAGE_CHARS).collect(),
    })
}

async fn json<T: DeserializeOwned>(response: Response) -> Result<T, Failure> {
    response.json::<T>().await.map_err(|err| {
        Failure::Store(GraphStoreError::BackendError {
            message: "unexpected HugeGraph response body".to_owned(),
            source: Some(Box::new(err)),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(status: StatusCode) -> GraphStoreError {
        Failure::Status {
            status,
            exception: "IllegalArgumentException".to_owned(),
            message: "boom".to_owned(),
        }
        .into()
    }

    #[test]
    fn statuses_map_to_store_error_categories() {
        assert!(matches!(
            status(StatusCode::CONFLICT),
            GraphStoreError::Conflict(_)
        ));
        assert!(matches!(
            status(StatusCode::SERVICE_UNAVAILABLE),
            GraphStoreError::BackendUnavailable { .. }
        ));
        assert!(matches!(
            status(StatusCode::BAD_REQUEST),
            GraphStoreError::BackendError { .. }
        ));
        assert!(matches!(
            status(StatusCode::NOT_FOUND),
            GraphStoreError::BackendError { .. }
        ));
        let internal = status(StatusCode::INTERNAL_SERVER_ERROR);
        assert!(matches!(internal, GraphStoreError::BackendError { .. }));
        assert!(
            internal
                .to_string()
                .contains("IllegalArgumentException: boom")
        );
    }

    #[test]
    fn urls_are_scoped_to_the_configured_graph() {
        let config = HugeGraphConfig::new("http://hugegraph:8080/proxy/")
            .unwrap()
            .with_graph_name("mindgraph")
            .unwrap();
        let client = Client::new(&config).unwrap();
        assert_eq!(
            client.url(&["graph", "vertices", "\"g:e\""]).as_str(),
            "http://hugegraph:8080/proxy/graphspaces/DEFAULT/graphs/mindgraph/graph/vertices/%22g:e%22"
        );
    }
}
