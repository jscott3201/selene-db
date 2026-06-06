//! Exact JSON containment search over graph node properties.
//!
//! This module is the JSON containment correctness oracle and small-corpus path.
//! It scans the current graph snapshot for JSON-valued node properties and
//! returns node candidates whose stored JSON recursively contains the requested
//! candidate value.

use std::collections::BinaryHeap;
use std::time::Duration;

use selene_core::{CancellationCause, CancellationChecker, DbString, JsonValue, NodeId, Value};

use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::shared::SharedGraph;
use crate::store::RowIndex;

pub(crate) const JSON_SEARCH_CANCEL_STRIDE: usize = 1024;

/// One JSON-containment node hit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonContainmentHit {
    /// Matched node id.
    pub node_id: NodeId,
}

/// Error returned by checked JSON containment search APIs.
#[derive(Debug, thiserror::Error)]
pub enum JsonSearchError {
    /// Graph storage or consistency failure.
    #[error(transparent)]
    Graph(#[from] GraphError),
    /// Caller requested cooperative cancellation.
    #[error("JSON search cancelled")]
    Cancelled,
    /// Statement deadline elapsed.
    #[error("JSON search timed out after {elapsed:?}")]
    Timeout {
        /// Wall-clock duration since the deadline elapsed.
        elapsed: Duration,
    },
}

impl JsonSearchError {
    fn into_graph_error(self) -> GraphError {
        match self {
            Self::Graph(error) => error,
            Self::Cancelled | Self::Timeout { .. } => GraphError::Inconsistent {
                reason: format!("disabled JSON-search checker returned {self}"),
            },
        }
    }
}

impl From<CancellationCause> for JsonSearchError {
    fn from(cause: CancellationCause) -> Self {
        match cause {
            CancellationCause::Cancelled => Self::Cancelled,
            CancellationCause::Timeout { elapsed } => Self::Timeout { elapsed },
        }
    }
}

impl SeleneGraph {
    /// Exhaustively find JSON-valued node properties containing `candidate`.
    pub fn exact_json_contains_nodes(
        &self,
        label: &DbString,
        property: &DbString,
        candidate: &JsonValue,
        k: usize,
    ) -> GraphResult<Vec<JsonContainmentHit>> {
        self.exact_json_contains_nodes_checked(
            label,
            property,
            candidate,
            k,
            CancellationChecker::disabled(),
        )
        .map_err(JsonSearchError::into_graph_error)
    }

    /// Exhaustively find JSON-valued node properties with cancellation checks.
    pub fn exact_json_contains_nodes_checked(
        &self,
        label: &DbString,
        property: &DbString,
        candidate: &JsonValue,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<JsonContainmentHit>, JsonSearchError> {
        checker.check()?;
        if k == 0 {
            return Ok(Vec::new());
        }
        let Some(label_rows) = self.nodes_with_label(label) else {
            return Ok(Vec::new());
        };

        let mut top_k = JsonContainmentTopK::new(k);
        let mut rows_since_check = 0usize;
        for raw_row in label_rows.iter() {
            rows_since_check += 1;
            if rows_since_check >= JSON_SEARCH_CANCEL_STRIDE {
                checker.check()?;
                rows_since_check = 0;
            }
            if !self.node_store.is_alive(raw_row) {
                continue;
            }
            let row = RowIndex::new(raw_row);
            let node_id = self
                .node_id_for_row(row)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "label index row {raw_row} for {} has no node id",
                        label.as_str()
                    ),
                })?;
            let properties = self
                .node_store
                .properties
                .get(raw_row as usize)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "JSON search row {raw_row} for {} has no property row",
                        label.as_str()
                    ),
                })?;
            let Some(Value::Json(value)) = properties.get(property) else {
                continue;
            };
            if value.contains(candidate) {
                top_k.push(node_id);
            }
        }
        Ok(top_k.into_hits())
    }
}

impl SharedGraph {
    /// Exhaustively find JSON-valued node properties in the current snapshot.
    pub fn exact_json_contains_nodes(
        &self,
        label: &DbString,
        property: &DbString,
        candidate: &JsonValue,
        k: usize,
    ) -> GraphResult<Vec<JsonContainmentHit>> {
        self.read()
            .exact_json_contains_nodes(label, property, candidate, k)
    }

    /// Exhaustively find JSON-valued node properties with cancellation checks.
    pub fn exact_json_contains_nodes_checked(
        &self,
        label: &DbString,
        property: &DbString,
        candidate: &JsonValue,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<JsonContainmentHit>, JsonSearchError> {
        self.read()
            .exact_json_contains_nodes_checked(label, property, candidate, k, checker)
    }
}

struct JsonContainmentTopK {
    k: usize,
    nodes: BinaryHeap<NodeId>,
}

impl JsonContainmentTopK {
    fn new(k: usize) -> Self {
        Self {
            k,
            nodes: BinaryHeap::new(),
        }
    }

    fn push(&mut self, node_id: NodeId) {
        if self.k == 0 {
            return;
        }
        if self.nodes.len() < self.k {
            self.nodes.push(node_id);
            return;
        }
        let Some(mut max_node_id) = self.nodes.peek_mut() else {
            return;
        };
        if node_id < *max_node_id {
            *max_node_id = node_id;
        }
    }

    fn into_hits(self) -> Vec<JsonContainmentHit> {
        self.nodes
            .into_sorted_vec()
            .into_iter()
            .map(|node_id| JsonContainmentHit { node_id })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use selene_core::{
        CancellationChecker, CancellationToken, GraphId, JsonValue, LabelSet, NodeId, PropertyMap,
        Value, db_string,
    };

    use crate::SharedGraph;

    fn label(value: &str) -> selene_core::DbString {
        db_string(value).expect("test string fits DB string cap")
    }

    fn json(value: serde_json::Value) -> Value {
        Value::Json(JsonValue::new(value).expect("JSON is valid"))
    }

    #[test]
    fn exact_json_contains_nodes_matches_nested_candidates() {
        let doc = label("Doc");
        let payload = label("payload");
        let graph = SharedGraph::new(GraphId::new(1));
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for value in [
            json(serde_json::json!({"memory": {"kind": "episodic", "score": 7}})),
            json(serde_json::json!({"memory": {"kind": "semantic"}})),
            Value::String(label("not json")),
            json(serde_json::json!(["agent", {"memory": {"kind": "episodic"}}])),
        ] {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    PropertyMap::from_pairs([(payload.clone(), value)])
                        .expect("properties are valid"),
                )
                .expect("node inserts");
        }
        txn.commit().expect("seed commits");

        let candidate =
            JsonValue::new(serde_json::json!({"memory": {"kind": "episodic"}})).unwrap();
        let hits = graph
            .exact_json_contains_nodes(&doc, &payload, &candidate, 10)
            .expect("search succeeds");

        assert_eq!(
            hits.into_iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
            vec![NodeId::new(1), NodeId::new(4)]
        );
    }

    #[test]
    fn exact_json_contains_nodes_observes_zero_k_and_cancellation() {
        let doc = label("Doc");
        let payload = label("payload");
        let graph = SharedGraph::new(GraphId::new(2));
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                PropertyMap::from_pairs([(
                    payload.clone(),
                    json(serde_json::json!({"kind": "memory"})),
                )])
                .expect("properties are valid"),
            )
            .expect("node inserts");
        txn.commit().expect("seed commits");
        let candidate = JsonValue::new(serde_json::json!({"kind": "memory"})).unwrap();

        assert!(
            graph
                .exact_json_contains_nodes(&doc, &payload, &candidate, 0)
                .expect("zero-k search succeeds")
                .is_empty()
        );

        let token = CancellationToken::new();
        token.cancel();
        let err = graph
            .exact_json_contains_nodes_checked(
                &doc,
                &payload,
                &candidate,
                10,
                CancellationChecker::new(Some(&token), None),
            )
            .expect_err("cancelled search reports cancellation");
        assert!(matches!(err, crate::JsonSearchError::Cancelled));
    }
}
