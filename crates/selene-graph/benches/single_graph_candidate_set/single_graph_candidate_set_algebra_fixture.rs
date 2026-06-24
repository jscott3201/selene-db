use selene_core::NodeId;
use selene_graph::{VectorCandidateSet, VectorNodeSearchHit};

#[derive(Clone, Debug)]
pub(super) struct VectorCandidateAlgebraFixture {
    left: VectorCandidateSet,
    right: VectorCandidateSet,
    hits: Vec<VectorNodeSearchHit>,
    left_width: usize,
    right_width: usize,
    overlap_width: usize,
}

impl VectorCandidateAlgebraFixture {
    pub(super) fn build(set_width: usize, overlap_width: usize) -> Self {
        let overlap_width = overlap_width.min(set_width);
        let right_start = set_width - overlap_width + 1;
        let left_nodes = (1..=set_width)
            .map(|id| NodeId::new(id as u64))
            .collect::<Vec<_>>();
        let right_nodes = (right_start..right_start + set_width)
            .map(|id| NodeId::new(id as u64))
            .collect::<Vec<_>>();
        let hits = right_nodes
            .iter()
            .enumerate()
            .map(|(offset, node_id)| VectorNodeSearchHit {
                node_id: *node_id,
                distance: offset as f64,
            })
            .collect::<Vec<_>>();
        Self {
            left: VectorCandidateSet::from_nodes(left_nodes),
            right: VectorCandidateSet::from_nodes(right_nodes),
            hits,
            left_width: set_width,
            right_width: set_width,
            overlap_width,
        }
    }

    pub(super) fn build_asymmetric(left_width: usize, right_width: usize) -> Self {
        let overlap_width = left_width.min(right_width);
        let left_nodes = (1..=left_width)
            .map(|id| NodeId::new(id as u64 * 64))
            .collect::<Vec<_>>();
        let right_nodes = (1..=right_width)
            .map(|id| NodeId::new(id as u64))
            .collect::<Vec<_>>();
        Self {
            left: VectorCandidateSet::from_nodes(left_nodes),
            right: VectorCandidateSet::from_nodes(right_nodes),
            hits: Vec::new(),
            left_width,
            right_width,
            overlap_width,
        }
    }

    pub(super) fn build_disjoint(set_width: usize) -> Self {
        let left_nodes = (1..=set_width)
            .map(|id| NodeId::new(id as u64))
            .collect::<Vec<_>>();
        let right_nodes = (set_width + 1..=set_width.saturating_mul(2))
            .map(|id| NodeId::new(id as u64))
            .collect::<Vec<_>>();
        Self {
            left: VectorCandidateSet::from_nodes(left_nodes),
            right: VectorCandidateSet::from_nodes(right_nodes),
            hits: Vec::new(),
            left_width: set_width,
            right_width: set_width,
            overlap_width: 0,
        }
    }

    pub(super) fn bench_id(&self, prefix: &str) -> String {
        format!(
            "{prefix}_l{}_r{}_o{}",
            self.left_width, self.right_width, self.overlap_width
        )
    }

    pub(super) const fn left(&self) -> &VectorCandidateSet {
        &self.left
    }

    pub(super) const fn right(&self) -> &VectorCandidateSet {
        &self.right
    }

    pub(super) fn hits(&self) -> &[VectorNodeSearchHit] {
        &self.hits
    }

    pub(super) const fn set_width(&self) -> usize {
        self.left_width
    }

    pub(super) const fn left_width(&self) -> usize {
        self.left_width
    }

    pub(super) const fn overlap_width(&self) -> usize {
        self.overlap_width
    }
}
