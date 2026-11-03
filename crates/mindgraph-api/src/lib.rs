//! HTTP API of MindGraph Open (ADR-006).
//!
//! Builds the Axum router served by the `mindgraph-core` binary. M0 exposes
//! only a liveness check. The versioned `/api/v1` routes, the stable error
//! envelope and request IDs arrive in M3.

mod config;

use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

pub use config::{Config, ConfigError};

/// The service name reported by the health check.
pub const SERVICE_NAME: &str = "mindgraph-core";

/// Response body of `GET /health`.
#[derive(Debug, Serialize)]
pub struct Health {
    /// Always `"ok"` when the process is serving requests.
    pub status: &'static str,
    /// The service name.
    pub service: &'static str,
    /// The crate version.
    pub version: &'static str,
}

/// Builds the application router.
pub fn router() -> Router {
    Router::new().route("/health", get(health))
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: SERVICE_NAME,
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    async fn get(path: &str) -> (StatusCode, Vec<u8>) {
        let response = router()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, body.to_vec())
    }

    #[tokio::test]
    async fn health_reports_ok() {
        let (status, body) = get("/health").await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "mindgraph-core");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn unknown_routes_return_not_found() {
        let (status, _) = get("/api/v1/graphs").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
