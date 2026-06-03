//! Exact native vector search over graph node properties.

use selene_core::{IStr, NodeId, Value, VectorMetric, VectorTopK, VectorValue};

use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::shared::SharedGraph;
use crate::store::RowIndex;

/// Exact vector-search result for a graph node.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorNodeSearchHit {
    /// Stable external node identifier.
    pub node_id: NodeId,
    /// Lower-is-better score under the requested [`VectorMetric`].
    pub distance: f64,
}

impl SeleneGraph {
    /// Exhaustively rank vector-valued node properties for one label.
    ///
    /// This is the correctness oracle and small-corpus path for future ANN
    /// indexes: it scans the row bitmap for `label`, skips nodes where
    /// `property` is absent or not a vector, and returns the exact best `k`
    /// matches. Graph structural inconsistencies are reported as
    /// [`GraphError::Inconsistent`]; vector metric errors such as dimension
    /// mismatch propagate through [`GraphError::Core`].
    pub fn exact_vector_search_nodes(
        &self,
        label: &IStr,
        property: &IStr,
        query: &VectorValue,
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        let Some(rows) = self.nodes_with_label(label) else {
            return Ok(Vec::new());
        };

        let mut top_k = VectorTopK::new(k);
        for raw_row in rows.iter() {
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
                        "label index row {raw_row} for {} has no property row",
                        label.as_str()
                    ),
                })?;
            let Some(Value::Vector(vector)) = properties.get(property) else {
                continue;
            };
            let distance = metric.distance(query, vector)?;
            top_k.push_distance(node_id, distance);
        }

        Ok(top_k
            .into_hits()
            .into_iter()
            .map(|hit| VectorNodeSearchHit {
                node_id: hit.key,
                distance: hit.distance,
            })
            .collect())
    }
}

impl SharedGraph {
    /// Exhaustively rank vector-valued node properties in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`SeleneGraph::exact_vector_search_nodes`], so the result is lock-free
    /// with respect to concurrent writers once the snapshot pointer is read.
    pub fn exact_vector_search_nodes(
        &self,
        label: &IStr,
        property: &IStr,
        query: &VectorValue,
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.read()
            .exact_vector_search_nodes(label, property, query, metric, k)
    }
}

#[cfg(test)]
mod tests {
    use selene_core::{
        CoreError, GraphId, LabelDiff, LabelSet, PropertyDiff, PropertyMap, Value, VectorValue,
        intern,
    };

    use super::*;

    fn vector(components: &[f32]) -> VectorValue {
        VectorValue::new(components.to_vec()).expect("test vector is valid")
    }

    fn props(key: &IStr, value: Value) -> PropertyMap {
        PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
    }

    #[test]
    fn exact_vector_search_ranks_labelled_vector_nodes() {
        let shared = SharedGraph::new(GraphId::new(91));
        let doc = intern("vector.doc").unwrap();
        let other = intern("vector.other").unwrap();
        let embedding = intern("embedding").unwrap();
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[2.0, 0.0]))),
                )
                .unwrap();
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
                )
                .unwrap();
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::String(intern("skip").unwrap())),
                )
                .unwrap();
            mutator
                .create_node(
                    LabelSet::single(other),
                    props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
                )
                .unwrap();
            txn.commit().unwrap();
        }

        let hits = shared
            .exact_vector_search_nodes(
                &doc,
                &embedding,
                &vector(&[0.0, 0.0]),
                VectorMetric::SquaredEuclidean,
                10,
            )
            .unwrap();

        assert_eq!(
            hits,
            vec![
                VectorNodeSearchHit {
                    node_id: NodeId::new(2),
                    distance: 1.0,
                },
                VectorNodeSearchHit {
                    node_id: NodeId::new(1),
                    distance: 4.0,
                },
            ]
        );
    }

    #[test]
    fn exact_vector_search_zero_k_and_missing_label_are_empty() {
        let shared = SharedGraph::new(GraphId::new(92));
        let doc = intern("vector.empty.doc").unwrap();
        let missing = intern("vector.empty.missing").unwrap();
        let embedding = intern("embedding").unwrap();
        {
            let mut txn = shared.begin_write();
            txn.mutator()
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[1.0]))),
                )
                .unwrap();
            txn.commit().unwrap();
        }

        assert!(
            shared
                .exact_vector_search_nodes(
                    &doc,
                    &embedding,
                    &vector(&[0.0]),
                    VectorMetric::SquaredEuclidean,
                    0,
                )
                .unwrap()
                .is_empty()
        );
        assert!(
            shared
                .exact_vector_search_nodes(
                    &missing,
                    &embedding,
                    &vector(&[0.0]),
                    VectorMetric::SquaredEuclidean,
                    10,
                )
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn exact_vector_search_uses_node_id_tie_breaks() {
        let shared = SharedGraph::new(GraphId::new(93));
        let doc = intern("vector.tie.doc").unwrap();
        let embedding = intern("embedding").unwrap();
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for _ in 0..3 {
                mutator
                    .create_node(
                        LabelSet::single(doc.clone()),
                        props(&embedding, Value::Vector(vector(&[1.0]))),
                    )
                    .unwrap();
            }
            txn.commit().unwrap();
        }

        let hits = shared
            .exact_vector_search_nodes(
                &doc,
                &embedding,
                &vector(&[0.0]),
                VectorMetric::SquaredEuclidean,
                2,
            )
            .unwrap();

        assert_eq!(
            hits,
            vec![
                VectorNodeSearchHit {
                    node_id: NodeId::new(1),
                    distance: 1.0,
                },
                VectorNodeSearchHit {
                    node_id: NodeId::new(2),
                    distance: 1.0,
                },
            ]
        );
    }

    #[test]
    fn exact_vector_search_tracks_update_and_delete_visibility() {
        let shared = SharedGraph::new(GraphId::new(94));
        let doc = intern("vector.visible.doc").unwrap();
        let embedding = intern("embedding").unwrap();
        let (first, second) = {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            let first = mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[10.0]))),
                )
                .unwrap();
            let second = mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[1.0]))),
                )
                .unwrap();
            txn.commit().unwrap();
            (first, second)
        };
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            mutator
                .update_node(
                    first,
                    LabelDiff::new([], []).unwrap(),
                    PropertyDiff::new([(embedding.clone(), Value::Vector(vector(&[0.25])))], [])
                        .unwrap(),
                )
                .unwrap();
            mutator.delete_node(second).unwrap();
            txn.commit().unwrap();
        }

        let hits = shared
            .exact_vector_search_nodes(
                &doc,
                &embedding,
                &vector(&[0.0]),
                VectorMetric::SquaredEuclidean,
                10,
            )
            .unwrap();

        assert_eq!(
            hits,
            vec![VectorNodeSearchHit {
                node_id: first,
                distance: 0.0625,
            }]
        );
    }

    #[test]
    fn exact_vector_search_surfaces_metric_errors() {
        let shared = SharedGraph::new(GraphId::new(95));
        let doc = intern("vector.error.doc").unwrap();
        let embedding = intern("embedding").unwrap();
        {
            let mut txn = shared.begin_write();
            txn.mutator()
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[1.0, 2.0]))),
                )
                .unwrap();
            txn.commit().unwrap();
        }

        let error = shared
            .exact_vector_search_nodes(
                &doc,
                &embedding,
                &vector(&[1.0]),
                VectorMetric::SquaredEuclidean,
                10,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            GraphError::Core(CoreError::VectorDimensionMismatch { lhs: 1, rhs: 2 })
        ));
    }

    #[test]
    fn exact_vector_search_surfaces_cosine_zero_norm() {
        let shared = SharedGraph::new(GraphId::new(96));
        let doc = intern("vector.cosine.doc").unwrap();
        let embedding = intern("embedding").unwrap();
        {
            let mut txn = shared.begin_write();
            txn.mutator()
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
                )
                .unwrap();
            txn.commit().unwrap();
        }

        let error = shared
            .exact_vector_search_nodes(
                &doc,
                &embedding,
                &vector(&[1.0, 0.0]),
                VectorMetric::Cosine,
                10,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            GraphError::Core(CoreError::VectorZeroNorm { side: "rhs" })
        ));
    }
}
