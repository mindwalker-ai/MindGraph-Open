//! Wire formats of the HugeGraph 1.7 REST API. Private to the adapter.
//!
//! Responses keep ids and properties as loose JSON so that unexpected data
//! surfaces as a precise `CorruptData` error in the codec instead of an
//! opaque deserialisation failure.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Body of `POST graph/vertices/batch` items.
#[derive(Debug, Serialize)]
pub(crate) struct NewVertex {
    pub(crate) id: String,
    pub(crate) label: &'static str,
    pub(crate) properties: Map<String, Value>,
}

/// Body of `POST graph/edges/batch` items.
#[derive(Debug, Serialize)]
pub(crate) struct NewEdge {
    pub(crate) label: &'static str,
    #[serde(rename = "outV")]
    pub(crate) out_v: String,
    #[serde(rename = "outVLabel")]
    pub(crate) out_v_label: &'static str,
    #[serde(rename = "inV")]
    pub(crate) in_v: String,
    #[serde(rename = "inVLabel")]
    pub(crate) in_v_label: &'static str,
    pub(crate) properties: Map<String, Value>,
}

/// A vertex as returned by HugeGraph.
#[derive(Debug, Deserialize)]
pub(crate) struct Vertex {
    pub(crate) id: Value,
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) properties: Map<String, Value>,
}

/// An edge as returned by HugeGraph.
#[derive(Debug, Deserialize)]
pub(crate) struct Edge {
    pub(crate) label: String,
    #[serde(rename = "outV")]
    pub(crate) out_v: Value,
    #[serde(rename = "inV")]
    pub(crate) in_v: Value,
    #[serde(default)]
    pub(crate) properties: Map<String, Value>,
}

/// `GET graph/vertices` and `GET traversers/vertices`.
#[derive(Debug, Deserialize)]
pub(crate) struct VertexPage {
    #[serde(default)]
    pub(crate) vertices: Vec<Vertex>,
    #[serde(default)]
    pub(crate) page: Option<String>,
}

/// `GET graph/edges` and `GET traversers/edgeexist`.
#[derive(Debug, Deserialize)]
pub(crate) struct EdgePage {
    #[serde(default)]
    pub(crate) edges: Vec<Edge>,
    #[serde(default)]
    pub(crate) page: Option<String>,
}

/// HugeGraph's error body: `{"exception": "...", "message": "...", ...}`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ErrorBody {
    #[serde(default)]
    pub(crate) exception: String,
    #[serde(default)]
    pub(crate) message: String,
}
