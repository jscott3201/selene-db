use selene_core::{CancellationChecker, DbString, VectorMetric, VectorValue};

use crate::error::GraphError;
use crate::vector_index::{HnswSearchScratch, VectorIndex};

use super::{
    ApproximateVectorSearchOptions, SeleneGraph, VectorCandidateSet, VectorIndexSearchHit,
    VectorNodeSearchHit, VectorSearchError, ann_row_hits_to_node_hits, rerank_ann_row_candidates,
};

pub(super) fn ann_index_batch_search(
    graph: &SeleneGraph,
    label: &DbString,
    index: &VectorIndex,
    queries: &[VectorValue],
    options: ApproximateVectorSearchOptions,
    query_dimension: u32,
    checker: CancellationChecker<'_>,
) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
    for query in queries {
        checker.check()?;
        let dimension = u32::try_from(query.dimension())
            .map_err(|_| VectorSearchError::ApproximateIndexMissing)?;
        if dimension != query_dimension {
            return Err(VectorSearchError::ApproximateIndexMissing);
        }
    }
    if index.is_ivf() {
        let row_batches = index
            .ivf_candidates_batch(queries, options.k, options.ef_search)
            .ok_or(VectorSearchError::ApproximateIndexMissing)?
            .map_err(GraphError::from)?;
        return ann_row_hit_batches_to_node_hits(graph, label, row_batches, &checker);
    }

    let mut scratch = HnswSearchScratch::default();
    let mut batch_hits = Vec::with_capacity(queries.len());
    for query in queries {
        checker.check()?;
        let row_hits = index
            .ann_search_with_scratch(query, options.k, options.ef_search, &mut scratch)
            .ok_or(VectorSearchError::ApproximateIndexMissing)?
            .map_err(GraphError::from)?;

        batch_hits.push(ann_row_hits_to_node_hits(graph, label, row_hits, &checker)?);
    }
    Ok(batch_hits)
}

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

pub(super) fn ann_row_hit_batches_to_node_hits(
    graph: &SeleneGraph,
    label: &DbString,
    row_batches: Vec<Vec<VectorIndexSearchHit>>,
    checker: &CancellationChecker<'_>,
) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
    let mut batch_hits = Vec::with_capacity(row_batches.len());
    for row_hits in row_batches {
        checker.check()?;
        batch_hits.push(ann_row_hits_to_node_hits(graph, label, row_hits, checker)?);
    }
    Ok(batch_hits)
}
