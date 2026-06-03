//! Reusable HNSW search and construction buffers.

use std::collections::BinaryHeap;

use rustc_hash::FxHashSet;

use super::candidate::{Candidate, MaxCandidate, MinCandidate};

/// Scratch buffers reused while searching, constructing, or repairing HNSW links.
#[derive(Debug, Default)]
pub(crate) struct HnswSearchScratch {
    pub(super) visited: FxHashSet<u32>,
    pub(super) candidates: BinaryHeap<MinCandidate>,
    pub(super) best: BinaryHeap<MaxCandidate>,
    pub(super) result: Vec<Candidate>,
    pub(super) prune_candidates: Vec<Candidate>,
    pub(super) fallback: Vec<u32>,
}

impl HnswSearchScratch {
    pub(super) fn reset_layer(&mut self, search_width: usize) {
        self.visited.clear();
        self.candidates.clear();
        self.best.clear();
        self.result.clear();
        self.visited.reserve(search_width);
        self.candidates.reserve(search_width);
        self.best.reserve(search_width);
        self.result.reserve(search_width);
    }

    pub(super) fn reset_prune(&mut self, candidate_count: usize) {
        self.prune_candidates.clear();
        self.prune_candidates.reserve(candidate_count);
    }
}
