use selene_core::{CancellationChecker, DbString, VectorMetric, VectorValue};

use super::{
    SeleneGraph, VectorCandidateSet, VectorIndexSearchHit, VectorNodeSearchHit, VectorSearchError,
    rerank_ann_row_candidates,
};

pub(super) fn rerank_ann_row_candidate_batches(
    graph: &SeleneGraph,
    property: &DbString,
    queries: &[VectorValue],
    metric: VectorMetric,
    k: usize,
    row_batches: Vec<Vec<VectorIndexSearchHit>>,
    checker: &CancellationChecker<'_>,
) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
    let mut batch_hits = Vec::with_capacity(queries.len());
    for (query, row_hits) in queries.iter().zip(row_batches) {
        batch_hits.push(rerank_ann_row_candidates(
            graph, property, query, metric, k, row_hits, checker,
        )?);
    }
    Ok(batch_hits)
}

pub(super) fn candidate_sets_match(lhs: &VectorCandidateSet, rhs: &VectorCandidateSet) -> bool {
    let lhs = lhs.as_nodes();
    let rhs = rhs.as_nodes();
    lhs.len() == rhs.len() && lhs.first() == rhs.first() && lhs.last() == rhs.last() && lhs == rhs
}
