use rayon::prelude::*;
use selene_core::{CancellationChecker, DbString, Value, VectorMetric, VectorTopK, VectorValue};

use crate::error::GraphError;
use crate::graph::SeleneGraph;
use crate::parallel_scan::should_parallelize_scan;

use super::{
    VECTOR_SEARCH_CANCEL_STRIDE, VectorCandidateSet, VectorNodeSearchHit, VectorSearchError,
    vector_node_hits,
};

#[cfg(not(test))]
const VECTOR_CANDIDATE_BATCH_PARALLEL_MIN_TOTAL_NODES: usize = 4096;
#[cfg(test)]
const VECTOR_CANDIDATE_BATCH_PARALLEL_MIN_TOTAL_NODES: usize = 8;

impl SeleneGraph {
    pub(super) fn score_vector_candidate_sets_batch_parallel(
        &self,
        property: &DbString,
        queries: &[VectorValue],
        candidate_sets: &[VectorCandidateSet],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        queries
            .par_iter()
            .zip(candidate_sets.par_iter())
            .map(|(query, candidates)| {
                checker.check()?;
                let scorer = metric.bind_query(query).map_err(GraphError::from)?;
                self.score_vector_candidate_set_serial(property, scorer, candidates, k, checker)
            })
            .collect()
    }

    pub(super) fn score_repeated_vector_candidate_set_batch_serial(
        &self,
        property: &DbString,
        queries: &[VectorValue],
        candidates: &VectorCandidateSet,
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        checker.check()?;
        if candidates.is_empty() {
            return Ok(vec![Vec::new(); queries.len()]);
        }

        let mut scorers = Vec::with_capacity(queries.len());
        for query in queries {
            scorers.push(metric.bind_query(query).map_err(GraphError::from)?);
        }

        let mut top_ks = (0..queries.len())
            .map(|_| VectorTopK::new(k))
            .collect::<Vec<_>>();
        for (offset, node_id) in candidates.as_nodes().iter().copied().enumerate() {
            if offset % VECTOR_SEARCH_CANCEL_STRIDE == 0 {
                checker.check()?;
            }
            let Some(properties) = self.node_properties(node_id) else {
                continue;
            };
            let Some(Value::Vector(vector)) = properties.get(property) else {
                continue;
            };
            for (scorer, top_k) in scorers.iter().zip(top_ks.iter_mut()) {
                let distance = scorer.distance(vector).map_err(GraphError::from)?;
                top_k.push_distance(node_id, distance);
            }
        }

        Ok(top_ks.into_iter().map(vector_node_hits).collect())
    }
}

pub(super) fn candidate_sets_all_match(candidate_sets: &[VectorCandidateSet]) -> bool {
    let Some(first) = candidate_sets.first() else {
        return false;
    };
    candidate_sets.len() > 1
        && candidate_sets
            .iter()
            .skip(1)
            .all(|candidates| candidate_sets_match(first, candidates))
}

pub(super) fn should_parallelize_candidate_batch_scoring(
    candidate_sets: &[VectorCandidateSet],
    k: usize,
) -> bool {
    if candidate_sets.len() <= 1 {
        return false;
    }
    let mut total_candidates = 0_usize;
    let mut max_candidates = 0_usize;
    let mut non_empty_sets = 0_usize;
    for candidate_count in candidate_sets.iter().map(VectorCandidateSet::len) {
        total_candidates += candidate_count;
        max_candidates = max_candidates.max(candidate_count);
        non_empty_sets += usize::from(candidate_count != 0);
    }
    if non_empty_sets <= 1 || max_candidates.saturating_mul(2) > total_candidates {
        return false;
    }

    should_parallelize_scan(
        total_candidates as u64,
        k,
        VECTOR_CANDIDATE_BATCH_PARALLEL_MIN_TOTAL_NODES as u64,
    )
}

fn candidate_sets_match(lhs: &VectorCandidateSet, rhs: &VectorCandidateSet) -> bool {
    let lhs = lhs.as_nodes();
    let rhs = rhs.as_nodes();
    lhs.len() == rhs.len() && lhs.first() == rhs.first() && lhs.last() == rhs.last() && lhs == rhs
}
