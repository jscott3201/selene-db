//! Public types for graph-level vector search and scoring.

use std::time::Duration;

use selene_core::{CancellationCause, IStr, NodeId, VectorMetric};

use crate::error::GraphError;

/// Exact vector-search result for a graph node.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorNodeSearchHit {
    /// Stable external node identifier.
    pub node_id: NodeId,
    /// Lower-is-better score under the requested [`VectorMetric`].
    pub distance: f64,
}

/// Canonical node candidates for vector scoring.
///
/// The set stores sorted, deduplicated [`NodeId`] values. It does not encode
/// vector property presence or liveness; scoring still applies normal snapshot
/// visibility and skips missing or non-vector candidates.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VectorCandidateSet {
    nodes: Vec<NodeId>,
}

impl VectorCandidateSet {
    /// Build a canonical candidate set from arbitrary node ids.
    #[must_use]
    pub fn from_nodes<I>(nodes: I) -> Self
    where
        I: IntoIterator<Item = NodeId>,
    {
        let mut nodes = nodes.into_iter().collect::<Vec<_>>();
        nodes.sort_unstable();
        nodes.dedup();
        Self { nodes }
    }

    /// Build a canonical candidate set from vector-search hits.
    #[must_use]
    pub fn from_search_hits<I, H>(hits: I) -> Self
    where
        I: IntoIterator<Item = H>,
        H: std::borrow::Borrow<VectorNodeSearchHit>,
    {
        Self::from_nodes(hits.into_iter().map(|hit| hit.borrow().node_id))
    }

    /// Return the sorted union of this set and `other`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let mut nodes = Vec::with_capacity(self.len().saturating_add(other.len()));
        let mut lhs = 0usize;
        let mut rhs = 0usize;
        while lhs < self.nodes.len() && rhs < other.nodes.len() {
            let left = self.nodes[lhs];
            let right = other.nodes[rhs];
            match left.cmp(&right) {
                std::cmp::Ordering::Less => {
                    nodes.push(left);
                    lhs += 1;
                }
                std::cmp::Ordering::Equal => {
                    nodes.push(left);
                    lhs += 1;
                    rhs += 1;
                }
                std::cmp::Ordering::Greater => {
                    nodes.push(right);
                    rhs += 1;
                }
            }
        }
        nodes.extend_from_slice(&self.nodes[lhs..]);
        nodes.extend_from_slice(&other.nodes[rhs..]);
        Self { nodes }
    }

    /// Return the sorted intersection of this set and `other`.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        let mut nodes = Vec::with_capacity(self.len().min(other.len()));
        let mut lhs = 0usize;
        let mut rhs = 0usize;
        while lhs < self.nodes.len() && rhs < other.nodes.len() {
            let left = self.nodes[lhs];
            let right = other.nodes[rhs];
            match left.cmp(&right) {
                std::cmp::Ordering::Less => {
                    lhs += 1;
                }
                std::cmp::Ordering::Equal => {
                    nodes.push(left);
                    lhs += 1;
                    rhs += 1;
                }
                std::cmp::Ordering::Greater => {
                    rhs += 1;
                }
            }
        }
        Self { nodes }
    }

    /// Return the sorted candidates in this set that are absent from `other`.
    #[must_use]
    pub fn difference(&self, other: &Self) -> Self {
        let mut nodes = Vec::with_capacity(self.len());
        let mut rhs_index = 0usize;
        for &node in &self.nodes {
            while rhs_index < other.nodes.len() && other.nodes[rhs_index] < node {
                rhs_index += 1;
            }
            if rhs_index == other.nodes.len() || other.nodes[rhs_index] != node {
                nodes.push(node);
            }
        }
        Self { nodes }
    }

    /// Borrow the sorted, deduplicated node ids.
    #[must_use]
    pub fn as_nodes(&self) -> &[NodeId] {
        &self.nodes
    }

    /// Consume the set and return the sorted, deduplicated node ids.
    #[must_use]
    pub fn into_nodes(self) -> Vec<NodeId> {
        self.nodes
    }

    /// Return the number of candidate nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Return true when the candidate set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl AsRef<[NodeId]> for VectorCandidateSet {
    fn as_ref(&self) -> &[NodeId] {
        self.as_nodes()
    }
}

/// Direction used when deriving vector-score candidates from a graph edge label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorNeighborDirection {
    /// Score outgoing neighbors reached from the anchor node.
    Outgoing,
    /// Score incoming neighbors that point at the anchor node.
    Incoming,
    /// Score both incoming and outgoing neighbors.
    Both,
}

/// Tunable options for deriving and ranking vector neighbors from graph edges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VectorNeighborSearchOptions<'a> {
    /// Edge label used to derive the one-hop candidate set.
    pub edge_label: &'a IStr,
    /// Direction used when walking one-hop graph adjacency.
    pub direction: VectorNeighborDirection,
    /// Distance metric requested by the caller.
    pub metric: VectorMetric,
    /// Maximum result count.
    pub k: usize,
}

impl<'a> VectorNeighborSearchOptions<'a> {
    /// Construct one-hop vector-neighbor scoring options.
    #[must_use]
    pub const fn new(
        edge_label: &'a IStr,
        direction: VectorNeighborDirection,
        metric: VectorMetric,
        k: usize,
    ) -> Self {
        Self {
            edge_label,
            direction,
            metric,
            k,
        }
    }
}

/// Tunable options for approximate vector search.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApproximateVectorSearchOptions {
    /// Distance metric requested by the caller.
    pub metric: VectorMetric,
    /// Maximum result count.
    pub k: usize,
    /// ANN search-width hint.
    pub ef_search: usize,
}

impl ApproximateVectorSearchOptions {
    /// Construct approximate vector-search options.
    #[must_use]
    pub const fn new(metric: VectorMetric, k: usize, ef_search: usize) -> Self {
        Self {
            metric,
            k,
            ef_search,
        }
    }
}

/// Error returned by cancellation-aware vector search.
#[derive(Debug, thiserror::Error)]
pub enum VectorSearchError {
    /// Graph storage or metric error.
    #[error(transparent)]
    Graph(#[from] GraphError),
    /// Caller-requested cancellation was observed.
    #[error("vector search cancelled")]
    Cancelled,
    /// Statement deadline elapsed while vector search was scanning.
    #[error("vector search deadline exceeded after {elapsed:?}")]
    Timeout {
        /// Duration since the deadline elapsed.
        elapsed: Duration,
    },
    /// Approximate search was requested without a matching ANN index.
    #[error("matching ANN vector index not found")]
    ApproximateIndexMissing,
    /// Approximate search metric does not match the ANN index metric.
    #[error("ANN vector index was built for {indexed:?}, not requested {requested:?}")]
    ApproximateMetricMismatch {
        /// Metric stored in the ANN index.
        indexed: VectorMetric,
        /// Metric requested by the caller.
        requested: VectorMetric,
    },
    /// Batched vector input lists do not align one-to-one.
    #[error(
        "batched vector input length mismatch: {queries} queries, {candidate_sets} candidate sets"
    )]
    BatchLengthMismatch {
        /// Query vector count.
        queries: usize,
        /// Candidate-set count.
        candidate_sets: usize,
    },
}

impl VectorSearchError {
    pub(crate) fn into_graph_error(self) -> GraphError {
        match self {
            Self::Graph(error) => error,
            Self::Cancelled | Self::Timeout { .. } => GraphError::Cancelled,
            Self::ApproximateIndexMissing
            | Self::ApproximateMetricMismatch { .. }
            | Self::BatchLengthMismatch { .. } => GraphError::Inconsistent {
                reason: self.to_string(),
            },
        }
    }
}

impl From<CancellationCause> for VectorSearchError {
    fn from(cause: CancellationCause) -> Self {
        match cause {
            CancellationCause::Cancelled => Self::Cancelled,
            CancellationCause::Timeout { elapsed } => Self::Timeout { elapsed },
        }
    }
}
