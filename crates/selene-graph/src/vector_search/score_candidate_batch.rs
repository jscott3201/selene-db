use rayon::prelude::*;
use selene_core::{
    CancellationChecker, DbString, NodeId, Value, VectorMetric, VectorMetricQuery, VectorTopK,
    VectorValue,
};

use crate::error::GraphError;
use crate::graph::SeleneGraph;
use crate::parallel_scan::{should_parallelize_scan, try_reduce_chunks};

use super::{
    VECTOR_SEARCH_CANCEL_STRIDE, VectorCandidateSet, VectorNodeSearchHit, VectorSearchError,
    vector_node_hits,
};

#[cfg(not(test))]
const VECTOR_CANDIDATE_BATCH_PARALLEL_MIN_TOTAL_NODES: usize = 4096;
#[cfg(test)]
const VECTOR_CANDIDATE_BATCH_PARALLEL_MIN_TOTAL_NODES: usize = 8;
#[cfg(not(test))]
const VECTOR_REPEATED_CANDIDATE_BATCH_PARALLEL_MIN_CANDIDATES: usize = 4096;
#[cfg(test)]
const VECTOR_REPEATED_CANDIDATE_BATCH_PARALLEL_MIN_CANDIDATES: usize = 4;
#[cfg(not(test))]
const VECTOR_REPEATED_CANDIDATE_BATCH_PARALLEL_CHUNK_NODES: usize = 32;
#[cfg(test)]
const VECTOR_REPEATED_CANDIDATE_BATCH_PARALLEL_CHUNK_NODES: usize = 2;
const VECTOR_CANDIDATE_BATCH_GROUP_MAX_SETS: usize = 128;

struct CandidateBatchScore<'a> {
    property: &'a DbString,
    queries: &'a [VectorValue],
    metric: VectorMetric,
    k: usize,
    checker: CancellationChecker<'a>,
}

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

    pub(super) fn score_repeated_vector_candidate_set_batch_parallel(
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

        let scorers = queries
            .iter()
            .map(|query| metric.bind_query(query).map_err(GraphError::from))
            .collect::<Result<Vec<_>, _>>()?;
        let top_ks = try_reduce_chunks(
            candidates.as_nodes(),
            VECTOR_REPEATED_CANDIDATE_BATCH_PARALLEL_CHUNK_NODES,
            checker,
            || new_batch_top_ks(queries.len(), k),
            |chunk| self.score_repeated_vector_candidate_set_chunk(property, &scorers, chunk, k),
            merge_batch_top_ks,
        )?;

        Ok(top_ks.into_iter().map(vector_node_hits).collect())
    }

    pub(super) fn score_vector_candidate_sets_batch_grouped_serial(
        &self,
        property: &DbString,
        queries: &[VectorValue],
        candidate_sets: &[VectorCandidateSet],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        let groups = repeated_candidate_set_groups(candidate_sets);
        if groups.is_empty() {
            let mut batch_hits = Vec::with_capacity(queries.len());
            for (query, candidates) in queries.iter().zip(candidate_sets) {
                checker.check()?;
                batch_hits.push(self.score_vector_candidate_set_checked(
                    property, query, candidates, metric, k, checker,
                )?);
            }
            return Ok(batch_hits);
        }

        let mut batch_hits = (0..queries.len()).map(|_| None).collect::<Vec<_>>();
        let score = CandidateBatchScore {
            property,
            queries,
            metric,
            k,
            checker,
        };
        for group in groups {
            let hits = self.score_repeated_vector_candidate_set_indexed_serial(
                &score,
                &group,
                &candidate_sets[group[0]],
            )?;
            for (query_index, hits) in group.into_iter().zip(hits) {
                batch_hits[query_index] = Some(hits);
            }
        }
        for (query_index, (query, candidates)) in queries.iter().zip(candidate_sets).enumerate() {
            if batch_hits[query_index].is_some() {
                continue;
            }
            checker.check()?;
            batch_hits[query_index] = Some(self.score_vector_candidate_set_checked(
                property, query, candidates, metric, k, checker,
            )?);
        }

        Ok(batch_hits
            .into_iter()
            .map(|hits| hits.expect("batched vector scoring fills every query slot"))
            .collect())
    }

    fn score_repeated_vector_candidate_set_indexed_serial(
        &self,
        score: &CandidateBatchScore<'_>,
        query_indices: &[usize],
        candidates: &VectorCandidateSet,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        score.checker.check()?;
        if candidates.is_empty() {
            return Ok(vec![Vec::new(); query_indices.len()]);
        }

        let mut scorers = Vec::with_capacity(query_indices.len());
        for query_index in query_indices {
            scorers.push(
                score
                    .metric
                    .bind_query(&score.queries[*query_index])
                    .map_err(GraphError::from)?,
            );
        }

        let mut top_ks = (0..query_indices.len())
            .map(|_| VectorTopK::new(score.k))
            .collect::<Vec<_>>();
        for (offset, node_id) in candidates.as_nodes().iter().copied().enumerate() {
            if offset % VECTOR_SEARCH_CANCEL_STRIDE == 0 {
                score.checker.check()?;
            }
            let Some(properties) = self.node_properties(node_id) else {
                continue;
            };
            let Some(Value::Vector(vector)) = properties.get(score.property) else {
                continue;
            };
            for (scorer, top_k) in scorers.iter().zip(top_ks.iter_mut()) {
                let distance = scorer.distance(vector).map_err(GraphError::from)?;
                top_k.push_distance(node_id, distance);
            }
        }

        Ok(top_ks.into_iter().map(vector_node_hits).collect())
    }

    fn score_repeated_vector_candidate_set_chunk(
        &self,
        property: &DbString,
        scorers: &[VectorMetricQuery<'_>],
        candidates: &[NodeId],
        k: usize,
    ) -> Result<Vec<VectorTopK<NodeId>>, VectorSearchError> {
        let mut top_ks = new_batch_top_ks(scorers.len(), k);
        for node_id in candidates.iter().copied() {
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
        Ok(top_ks)
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

pub(super) fn should_parallelize_repeated_candidate_batch(
    query_count: usize,
    candidate_count: usize,
    k: usize,
) -> bool {
    query_count > 1
        && candidate_count >= VECTOR_REPEATED_CANDIDATE_BATCH_PARALLEL_MIN_CANDIDATES
        && should_parallelize_scan(
            query_count.saturating_mul(candidate_count) as u64,
            k,
            VECTOR_CANDIDATE_BATCH_PARALLEL_MIN_TOTAL_NODES as u64,
        )
}

fn candidate_sets_match(lhs: &VectorCandidateSet, rhs: &VectorCandidateSet) -> bool {
    let lhs = lhs.as_nodes();
    let rhs = rhs.as_nodes();
    lhs.len() == rhs.len() && lhs.first() == rhs.first() && lhs.last() == rhs.last() && lhs == rhs
}

fn repeated_candidate_set_groups(candidate_sets: &[VectorCandidateSet]) -> Vec<Vec<usize>> {
    if candidate_sets.len() <= 2 || candidate_sets.len() > VECTOR_CANDIDATE_BATCH_GROUP_MAX_SETS {
        return Vec::new();
    }
    let mut assigned = vec![false; candidate_sets.len()];
    let mut groups = Vec::new();
    for index in 0..candidate_sets.len() {
        if assigned[index] {
            continue;
        }
        let mut group = Vec::new();
        for next in index + 1..candidate_sets.len() {
            if !assigned[next]
                && candidate_sets_match(&candidate_sets[index], &candidate_sets[next])
            {
                if group.is_empty() {
                    group.push(index);
                    assigned[index] = true;
                }
                group.push(next);
                assigned[next] = true;
            }
        }
        if group.len() > 1 {
            groups.push(group);
        }
    }
    groups
}

fn new_batch_top_ks(query_count: usize, k: usize) -> Vec<VectorTopK<NodeId>> {
    (0..query_count).map(|_| VectorTopK::new(k)).collect()
}

fn merge_batch_top_ks(
    mut lhs: Vec<VectorTopK<NodeId>>,
    rhs: Vec<VectorTopK<NodeId>>,
) -> Result<Vec<VectorTopK<NodeId>>, VectorSearchError> {
    debug_assert_eq!(lhs.len(), rhs.len());
    for (lhs_top_k, rhs_top_k) in lhs.iter_mut().zip(rhs) {
        for hit in rhs_top_k.into_hits() {
            lhs_top_k.push_distance(hit.key, hit.distance);
        }
    }
    Ok(lhs)
}
