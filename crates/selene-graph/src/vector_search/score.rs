use selene_core::{
    CancellationChecker, IStr, NodeId, Value, VectorMetric, VectorTopK, VectorValue,
};

use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::shared::SharedGraph;

use super::{
    VECTOR_SEARCH_CANCEL_STRIDE, VectorNodeSearchHit, VectorSearchError, vector_node_hits,
};

impl SeleneGraph {
    /// Score an explicit node candidate set against one query vector.
    ///
    /// This is the graph-retrieval rerank primitive: callers can produce
    /// candidates from graph pattern matches, graph algorithms, or ANN indexes,
    /// then rank only those nodes by a vector-valued property. Candidate ids are
    /// deduplicated before scoring. Missing, deleted, and non-vector candidates
    /// are skipped to match normal live-snapshot visibility.
    pub fn score_vector_nodes(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &[NodeId],
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.score_vector_nodes_checked(
            property,
            query,
            candidates,
            metric,
            k,
            CancellationChecker::disabled(),
        )
        .map_err(VectorSearchError::into_graph_error)
    }

    /// Score explicit node candidates with cancellation checks.
    ///
    /// This preserves [`Self::score_vector_nodes`] ordering and visibility while
    /// checking `checker` before work begins and every 1024 unique candidates.
    pub fn score_vector_nodes_checked(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &[NodeId],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        checker.check()?;
        if k == 0 || candidates.is_empty() {
            return Ok(Vec::new());
        }

        let mut unique_candidates = candidates.to_vec();
        unique_candidates.sort_unstable();
        unique_candidates.dedup();
        checker.check()?;

        let scorer = metric.bind_query(query).map_err(GraphError::from)?;
        let mut top_k = VectorTopK::new(k);
        for (offset, node_id) in unique_candidates.into_iter().enumerate() {
            if offset % VECTOR_SEARCH_CANCEL_STRIDE == 0 {
                checker.check()?;
            }
            let Some(properties) = self.node_properties(node_id) else {
                continue;
            };
            let Some(Value::Vector(vector)) = properties.get(property) else {
                continue;
            };
            let distance = scorer.distance(vector).map_err(GraphError::from)?;
            top_k.push_distance(node_id, distance);
        }

        Ok(vector_node_hits(top_k))
    }
}

impl SharedGraph {
    /// Score explicit node candidates in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`SeleneGraph::score_vector_nodes`].
    pub fn score_vector_nodes(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &[NodeId],
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.read()
            .score_vector_nodes(property, query, candidates, metric, k)
    }

    /// Lock-free read snapshot wrapper for
    /// [`SeleneGraph::score_vector_nodes_checked`].
    pub fn score_vector_nodes_checked(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &[NodeId],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        self.read()
            .score_vector_nodes_checked(property, query, candidates, metric, k, checker)
    }
}
