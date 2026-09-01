use rayon::prelude::*;
use selene_core::{CancellationChecker, DbString, NodeId};

use crate::graph::SeleneGraph;
use crate::parallel_scan::should_parallelize_scan;

use super::{VectorCandidateSet, VectorNeighborDirection, VectorSearchError};

#[cfg(not(test))]
const VECTOR_NEIGHBOR_BATCH_PARALLEL_MIN_ANCHORS: usize = 16;
#[cfg(test)]
const VECTOR_NEIGHBOR_BATCH_PARALLEL_MIN_ANCHORS: usize = 2;

#[cfg(not(test))]
const VECTOR_NEIGHBOR_BATCH_PARALLEL_MIN_CANDIDATES: usize = 8192;
#[cfg(test)]
const VECTOR_NEIGHBOR_BATCH_PARALLEL_MIN_CANDIDATES: usize = 8;

const VECTOR_NEIGHBOR_BATCH_PARALLEL_ESTIMATE_ANCHORS: usize = 4;

impl SeleneGraph {
    pub(super) fn vector_neighbor_candidate_sets_batch(
        &self,
        anchors: &[NodeId],
        edge_label: &DbString,
        direction: VectorNeighborDirection,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorCandidateSet>, VectorSearchError> {
        let live_anchors = self.bind_node_candidates(anchors.iter().copied())?;
        if let Some(first_anchor) = anchors.first()
            && anchors.iter().all(|anchor| anchor == first_anchor)
        {
            checker.note_nodes_scanned(1)?;
            let candidates = if live_anchors.contains(*first_anchor) {
                self.vector_neighbor_candidates(*first_anchor, edge_label, direction)
            } else {
                VectorCandidateSet::default()
            };
            return Ok(vec![candidates; anchors.len()]);
        }

        if self.should_parallelize_neighbor_candidate_batch(anchors, edge_label, direction, k) {
            checker.note_nodes_scanned(anchors.len())?;
            return anchors
                .par_iter()
                .map(|anchor| {
                    checker.check()?;
                    Ok(if live_anchors.contains(*anchor) {
                        self.vector_neighbor_candidates(*anchor, edge_label, direction)
                    } else {
                        VectorCandidateSet::default()
                    })
                })
                .collect();
        }

        let mut candidate_sets = Vec::with_capacity(anchors.len());
        let mut anchors_since_check = 0usize;
        for anchor in anchors {
            anchors_since_check += 1;
            if anchors_since_check >= super::VECTOR_SEARCH_CANCEL_STRIDE {
                checker.note_nodes_scanned(anchors_since_check)?;
                anchors_since_check = 0;
            }
            candidate_sets.push(if live_anchors.contains(*anchor) {
                self.vector_neighbor_candidates(*anchor, edge_label, direction)
            } else {
                VectorCandidateSet::default()
            });
        }
        if anchors_since_check > 0 {
            checker.note_nodes_scanned(anchors_since_check)?;
        }
        Ok(candidate_sets)
    }

    fn should_parallelize_neighbor_candidate_batch(
        &self,
        anchors: &[NodeId],
        edge_label: &DbString,
        direction: VectorNeighborDirection,
        k: usize,
    ) -> bool {
        if !should_parallelize_scan(
            anchors.len() as u64,
            k,
            VECTOR_NEIGHBOR_BATCH_PARALLEL_MIN_ANCHORS as u64,
        ) {
            return false;
        }
        let sample_count = anchors
            .len()
            .min(VECTOR_NEIGHBOR_BATCH_PARALLEL_ESTIMATE_ANCHORS);
        let sampled_candidates = anchors
            .iter()
            .take(sample_count)
            .map(|anchor| self.neighbor_candidate_work_estimate(*anchor, edge_label, direction))
            .sum::<usize>();
        let estimated_candidates = sampled_candidates
            .saturating_mul(anchors.len())
            .div_ceil(sample_count);

        estimated_candidates >= VECTOR_NEIGHBOR_BATCH_PARALLEL_MIN_CANDIDATES
    }

    fn neighbor_candidate_work_estimate(
        &self,
        anchor: NodeId,
        edge_label: &DbString,
        direction: VectorNeighborDirection,
    ) -> usize {
        let mut candidate_count = 0_usize;
        if matches!(
            direction,
            VectorNeighborDirection::Outgoing | VectorNeighborDirection::Both
        ) && let Some(entry) = self.outgoing_edges(anchor)
        {
            candidate_count += entry.iter_label(edge_label).count();
        }
        if matches!(
            direction,
            VectorNeighborDirection::Incoming | VectorNeighborDirection::Both
        ) && let Some(entry) = self.incoming_edges(anchor)
        {
            candidate_count += entry.iter_label(edge_label).count();
        }
        candidate_count
    }
}
