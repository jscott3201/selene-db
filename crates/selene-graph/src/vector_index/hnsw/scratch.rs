//! Reusable HNSW search and construction buffers.

use std::collections::BinaryHeap;

use super::candidate::{Candidate, MaxCandidate, MinCandidate};

/// Scratch buffers reused while searching, constructing, or repairing HNSW links.
#[derive(Debug, Default)]
pub(crate) struct HnswSearchScratch {
    pub(super) visited: VisitedEpochs,
    pub(super) candidates: BinaryHeap<MinCandidate>,
    pub(super) best: BinaryHeap<MaxCandidate>,
    pub(super) result: Vec<Candidate>,
    pub(super) prune_candidates: Vec<Candidate>,
    pub(super) fallback: Vec<u32>,
}

impl HnswSearchScratch {
    pub(super) fn reset_layer(&mut self, node_count: usize, search_width: usize) {
        self.visited.reset(node_count);
        self.candidates.clear();
        self.best.clear();
        self.result.clear();
        self.candidates.reserve(search_width);
        self.best.reserve(search_width);
        self.result.reserve(search_width);
    }

    pub(super) fn reset_prune(&mut self, candidate_count: usize) {
        self.prune_candidates.clear();
        self.prune_candidates.reserve(candidate_count);
    }
}

/// Dense epoch-marked visited state for HNSW layer walks.
#[derive(Debug, Default)]
pub(super) struct VisitedEpochs {
    marks: Vec<u32>,
    epoch: u32,
}

impl VisitedEpochs {
    pub(super) fn reset(&mut self, node_count: usize) {
        if self.epoch == u32::MAX {
            self.marks.fill(0);
            self.epoch = 1;
        } else {
            self.epoch += 1;
        }
        if self.marks.len() < node_count {
            self.marks.resize(node_count, 0);
        }
    }

    pub(super) fn visit(&mut self, node_id: u32) -> bool {
        let index = node_id as usize;
        debug_assert!(index < self.marks.len());
        let mark = &mut self.marks[index];
        if *mark == self.epoch {
            return false;
        }
        *mark = self.epoch;
        true
    }

    #[cfg(test)]
    pub(super) fn capacity(&self) -> usize {
        self.marks.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::VisitedEpochs;

    #[test]
    fn visited_epochs_rejects_repeat_within_epoch() {
        let mut visited = VisitedEpochs::default();
        visited.reset(4);

        assert!(visited.visit(2));
        assert!(!visited.visit(2));

        visited.reset(4);
        assert!(visited.visit(2));
    }

    #[test]
    fn visited_epochs_wraps_by_clearing_marks() {
        let mut visited = VisitedEpochs {
            marks: vec![u32::MAX; 3],
            epoch: u32::MAX,
        };

        visited.reset(3);

        assert_eq!(visited.epoch, 1);
        assert!(visited.visit(1));
        assert!(!visited.visit(1));
    }
}
