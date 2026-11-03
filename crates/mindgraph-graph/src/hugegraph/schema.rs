//! Idempotent bootstrap of MindGraph's physical HugeGraph schema.
//!
//! The schema is generic: one vertex label and one edge label for every
//! graph project, whatever its ontology. User entity and relationship types
//! are stored in `mg_type` and stay a MindGraph domain concept.
//!
//! Every element is created with `check_exist: false`. HugeGraph then
//! returns an existing element with the same definition unchanged, and
//! rejects a differing one with `ExistedException`.

use serde_json::{Value, json};

use super::client::{Client, Failure};
use super::codec::{EDGE_LABEL, PROP_GRAPH_ID, PROP_ID, PROP_PROPERTIES, PROP_TYPE, VERTEX_LABEL};
use crate::GraphStoreError;

/// `(endpoint, element name, definition)` in creation order.
fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    let mut definitions: Vec<(&'static str, &'static str, Value)> =
        [PROP_GRAPH_ID, PROP_TYPE, PROP_ID, PROP_PROPERTIES]
            .into_iter()
            .map(|name| {
                (
                    "propertykeys",
                    name,
                    json!({
                        "name": name,
                        "data_type": "TEXT",
                        "cardinality": "SINGLE",
                        "check_exist": false,
                    }),
                )
            })
            .collect();
    definitions.push((
        "vertexlabels",
        VERTEX_LABEL,
        json!({
            "name": VERTEX_LABEL,
            "id_strategy": "CUSTOMIZE_STRING",
            "properties": [PROP_GRAPH_ID, PROP_TYPE, PROP_PROPERTIES],
            "nullable_keys": [],
            "enable_label_index": true,
            "check_exist": false,
        }),
    ));
    definitions.push((
        "edgelabels",
        EDGE_LABEL,
        json!({
            "name": EDGE_LABEL,
            "source_label": VERTEX_LABEL,
            "target_label": VERTEX_LABEL,
            // MULTIPLE + sort key mg_id keeps parallel relationships apart.
            // With the default SINGLE frequency HugeGraph would silently
            // merge two relationships between the same entities.
            "frequency": "MULTIPLE",
            "sort_keys": [PROP_ID],
            "properties": [PROP_ID, PROP_GRAPH_ID, PROP_TYPE, PROP_PROPERTIES],
            "nullable_keys": [],
            "enable_label_index": true,
            "check_exist": false,
        }),
    ));
    // Graph isolation and type lookup. A prefix query on `mg_graph_id` alone
    // uses the same index.
    definitions.push((
        "indexlabels",
        "mindgraph_entity_by_graph_type",
        json!({
            "name": "mindgraph_entity_by_graph_type",
            "base_type": "VERTEX_LABEL",
            "base_value": VERTEX_LABEL,
            "index_type": "SECONDARY",
            "fields": [PROP_GRAPH_ID, PROP_TYPE],
            "check_exist": false,
        }),
    ));
    // Relationship id lookup, used to detect duplicates.
    definitions.push((
        "indexlabels",
        "mindgraph_relationship_by_graph_id",
        json!({
            "name": "mindgraph_relationship_by_graph_id",
            "base_type": "EDGE_LABEL",
            "base_value": EDGE_LABEL,
            "index_type": "SECONDARY",
            "fields": [PROP_GRAPH_ID, PROP_ID],
            "check_exist": false,
        }),
    ));
    definitions
}

/// Creates any missing schema element. Safe to call repeatedly.
pub(crate) async fn ensure(client: &Client) -> Result<(), GraphStoreError> {
    for (endpoint, name, definition) in definitions() {
        match client
            .post::<_, Value>(&["schema", endpoint], &[], &definition)
            .await
        {
            Ok(_) => {}
            Err(Failure::Status { exception, .. }) if exception == "ExistedException" => {
                return Err(GraphStoreError::Unsupported(format!(
                    "HugeGraph already defines `{name}` differently from MindGraph's physical \
                     schema; use a dedicated HugeGraph graph for MindGraph"
                )));
            }
            Err(failure) => return Err(failure.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_generic_and_complete() {
        let definitions = definitions();
        let names: Vec<&str> = definitions.iter().map(|(_, name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "mg_graph_id",
                "mg_type",
                "mg_id",
                "mg_properties",
                "mindgraph_entity",
                "mindgraph_relationship",
                "mindgraph_entity_by_graph_type",
                "mindgraph_relationship_by_graph_id",
            ]
        );
        assert!(
            definitions
                .iter()
                .all(|(_, _, body)| body["check_exist"] == json!(false)),
            "bootstrap must be idempotent"
        );
        let (_, _, edge) = &definitions[5];
        assert_eq!(edge["frequency"], "MULTIPLE");
        assert_eq!(edge["sort_keys"], json!(["mg_id"]));
    }
}
