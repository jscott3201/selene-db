//! Candidate ordering helpers for HNSW search heaps.

use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
pub(super) struct Candidate {
    pub(super) id: u32,
    pub(super) distance: f64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MinCandidate {
    pub(super) id: u32,
    pub(super) distance: f64,
}

impl MinCandidate {
    pub(super) const fn new(id: u32, distance: f64) -> Self {
        Self { id, distance }
    }
}

impl Eq for MinCandidate {}

impl PartialEq for MinCandidate {
    fn eq(&self, rhs: &Self) -> bool {
        self.id == rhs.id && self.distance.to_bits() == rhs.distance.to_bits()
    }
}

impl Ord for MinCandidate {
    fn cmp(&self, rhs: &Self) -> Ordering {
        rhs.distance
            .total_cmp(&self.distance)
            .then_with(|| rhs.id.cmp(&self.id))
    }
}

impl PartialOrd for MinCandidate {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MaxCandidate {
    pub(super) id: u32,
    pub(super) distance: f64,
}

impl MaxCandidate {
    pub(super) const fn new(id: u32, distance: f64) -> Self {
        Self { id, distance }
    }
}

impl Eq for MaxCandidate {}

impl PartialEq for MaxCandidate {
    fn eq(&self, rhs: &Self) -> bool {
        self.id == rhs.id && self.distance.to_bits() == rhs.distance.to_bits()
    }
}

impl Ord for MaxCandidate {
    fn cmp(&self, rhs: &Self) -> Ordering {
        self.distance
            .total_cmp(&rhs.distance)
            .then_with(|| self.id.cmp(&rhs.id))
    }
}

impl PartialOrd for MaxCandidate {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

pub(super) fn compare_candidate(lhs: &Candidate, rhs: &Candidate) -> Ordering {
    lhs.distance
        .total_cmp(&rhs.distance)
        .then_with(|| lhs.id.cmp(&rhs.id))
}

pub(super) fn closer(
    candidate_distance: f64,
    candidate: u32,
    current_distance: f64,
    current: u32,
) -> bool {
    candidate_distance
        .total_cmp(&current_distance)
        .then_with(|| candidate.cmp(&current))
        .is_lt()
}
