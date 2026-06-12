use rayon::prelude::*;
use selene_core::{CancellationChecker, DbString};

use crate::graph::SeleneGraph;
use crate::parallel_scan::should_parallelize_scan;

use super::{VectorCandidateSet, VectorNeighborDirection, VectorSearchError};

#[cfg(not(test))]
const VECTOR_EXPANDED_BATCH_PARALLEL_MIN_SETS: usize = 16;
#[cfg(test)]
const VECTOR_EXPANDED_BATCH_PARALLEL_MIN_SETS: usize = 2;

#[cfg(not(test))]
const VECTOR_EXPANDED_BATCH_PARALLEL_MIN_CANDIDATES: usize = 8192;
#[cfg(test)]
const VECTOR_EXPANDED_BATCH_PARALLEL_MIN_CANDIDATES: usize = 8;

const VECTOR_EXPANDED_BATCH_PARALLEL_ESTIMATE_SETS: usize = 4;
const VECTOR_EXPANDED_BATCH_GROUP_MAX_SETS: usize = 128;

impl SeleneGraph {
    pub(super) fn expand_vector_candidate_sets_batch(
        &self,
        root_sets: &[VectorCandidateSet],
        edge_label: &DbString,
        direction: VectorNeighborDirection,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorCandidateSet>, VectorSearchError> {
        if let Some(first_roots) = root_sets.first()
            && root_sets
                .iter()
                .skip(1)
                .all(|roots| candidate_sets_match(first_roots, roots))
        {
            checker.check()?;
            let expanded = self.expand_vector_candidate_set_checked(
                first_roots,
                edge_label,
                direction,
                checker,
            )?;
            return Ok(vec![expanded; root_sets.len()]);
        }

        let groups = repeated_root_set_groups(root_sets);
        if !groups.is_empty() {
            return self.expand_vector_candidate_sets_batch_grouped(
                root_sets, edge_label, direction, k, checker, groups,
            );
        }

        if self.should_parallelize_expanded_candidate_batch(root_sets, edge_label, direction, k) {
            return root_sets
                .par_iter()
                .map(|roots| {
                    checker.check()?;
                    self.expand_vector_candidate_set_checked(roots, edge_label, direction, checker)
                })
                .collect();
        }

        let mut expanded_sets = Vec::with_capacity(root_sets.len());
        for roots in root_sets {
            checker.check()?;
            expanded_sets.push(
                self.expand_vector_candidate_set_checked(roots, edge_label, direction, checker)?,
            );
        }
        Ok(expanded_sets)
    }

    fn expand_vector_candidate_sets_batch_grouped(
        &self,
        root_sets: &[VectorCandidateSet],
        edge_label: &DbString,
        direction: VectorNeighborDirection,
        k: usize,
        checker: CancellationChecker<'_>,
        groups: Vec<Vec<usize>>,
    ) -> Result<Vec<VectorCandidateSet>, VectorSearchError> {
        let mut expanded_sets = vec![None; root_sets.len()];
        let mut grouped = vec![false; root_sets.len()];
        for group in groups {
            checker.check()?;
            let expanded = self.expand_vector_candidate_set_checked(
                &root_sets[group[0]],
                edge_label,
                direction,
                checker,
            )?;
            for index in group {
                grouped[index] = true;
                expanded_sets[index] = Some(expanded.clone());
            }
        }

        let ungrouped = grouped
            .iter()
            .enumerate()
            .filter_map(|(index, is_grouped)| (!is_grouped).then_some(index))
            .collect::<Vec<_>>();
        let expanded_ungrouped = if self
            .should_parallelize_expanded_candidate_batch(root_sets, edge_label, direction, k)
        {
            ungrouped
                .par_iter()
                .map(|&index| {
                    checker.check()?;
                    self.expand_vector_candidate_set_checked(
                        &root_sets[index],
                        edge_label,
                        direction,
                        checker,
                    )
                    .map(|expanded| (index, expanded))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut expanded = Vec::with_capacity(ungrouped.len());
            for index in ungrouped {
                checker.check()?;
                expanded.push((
                    index,
                    self.expand_vector_candidate_set_checked(
                        &root_sets[index],
                        edge_label,
                        direction,
                        checker,
                    )?,
                ));
            }
            expanded
        };
        for (index, expanded) in expanded_ungrouped {
            expanded_sets[index] = Some(expanded);
        }

        Ok(expanded_sets
            .into_iter()
            .map(|expanded| expanded.expect("batched expansion fills every root slot"))
            .collect())
    }

    fn should_parallelize_expanded_candidate_batch(
        &self,
        root_sets: &[VectorCandidateSet],
        edge_label: &DbString,
        direction: VectorNeighborDirection,
        k: usize,
    ) -> bool {
        if !should_parallelize_scan(
            root_sets.len() as u64,
            k,
            VECTOR_EXPANDED_BATCH_PARALLEL_MIN_SETS as u64,
        ) {
            return false;
        }
        let sample_count = root_sets
            .len()
            .min(VECTOR_EXPANDED_BATCH_PARALLEL_ESTIMATE_SETS);
        let sampled_candidates = root_sets
            .iter()
            .take(sample_count)
            .map(|roots| self.expanded_candidate_work_estimate(roots, edge_label, direction))
            .sum::<usize>();
        let estimated_candidates = sampled_candidates
            .saturating_mul(root_sets.len())
            .div_ceil(sample_count);

        estimated_candidates >= VECTOR_EXPANDED_BATCH_PARALLEL_MIN_CANDIDATES
    }

    fn expanded_candidate_work_estimate(
        &self,
        roots: &VectorCandidateSet,
        edge_label: &DbString,
        direction: VectorNeighborDirection,
    ) -> usize {
        let mut candidate_count = roots.len();
        for root in roots.as_nodes().iter().copied() {
            if matches!(
                direction,
                VectorNeighborDirection::Outgoing | VectorNeighborDirection::Both
            ) && let Some(entry) = self.outgoing_edges(root)
            {
                candidate_count += entry.iter_label(edge_label).count();
            }
            if matches!(
                direction,
                VectorNeighborDirection::Incoming | VectorNeighborDirection::Both
            ) && let Some(entry) = self.incoming_edges(root)
            {
                candidate_count += entry.iter_label(edge_label).count();
            }
        }
        candidate_count
    }
}

fn candidate_sets_match(lhs: &VectorCandidateSet, rhs: &VectorCandidateSet) -> bool {
    let lhs = lhs.as_nodes();
    let rhs = rhs.as_nodes();
    lhs.len() == rhs.len() && lhs.first() == rhs.first() && lhs.last() == rhs.last() && lhs == rhs
}

fn repeated_root_set_groups(root_sets: &[VectorCandidateSet]) -> Vec<Vec<usize>> {
    if root_sets.len() <= 2 || root_sets.len() > VECTOR_EXPANDED_BATCH_GROUP_MAX_SETS {
        return Vec::new();
    }
    let mut assigned = vec![false; root_sets.len()];
    let mut groups = Vec::new();
    for index in 0..root_sets.len() {
        if assigned[index] {
            continue;
        }
        let mut group = Vec::new();
        for next in index + 1..root_sets.len() {
            if !assigned[next] && candidate_sets_match(&root_sets[index], &root_sets[next]) {
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
