//! Read-only HNSW graph storage.

use std::collections::HashMap;
use std::sync::Arc;

use selene_core::NodeId;
use smallvec::SmallVec;

use crate::VectorError;

use super::InternalIndex;

/// One indexed vector and its per-layer HNSW neighbor lists.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HnswNode {
    /// Source graph node keyed by this vector.
    pub node_id: NodeId,
    /// Dense f32 vector payload.
    pub vector: Arc<[f32]>,
    /// Neighbor indexes for each layer from 0 through [`Self::max_layer`].
    pub neighbors: SmallVec<[Vec<InternalIndex>; 4]>,
    /// Highest HNSW layer containing this node.
    pub max_layer: u8,
}

impl HnswNode {
    /// Construct a node with empty neighbor lists for layers `0..=max_layer`.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidNodeId`] when `node_id` is the reserved
    /// tombstone sentinel.
    pub fn new(node_id: NodeId, vector: Arc<[f32]>, max_layer: u8) -> Result<Self, VectorError> {
        if node_id == NodeId::TOMBSTONE {
            return Err(VectorError::InvalidNodeId {
                node_id,
                reason: "NodeId::TOMBSTONE cannot be added to an HNSW index".into(),
            });
        }
        debug_assert_ne!(
            node_id,
            NodeId::TOMBSTONE,
            "tombstone caught at debug tripwire"
        );

        let mut neighbors = SmallVec::with_capacity(usize::from(max_layer) + 1);
        for _ in 0..=max_layer {
            neighbors.push(Vec::new());
        }

        Ok(Self {
            node_id,
            vector,
            neighbors,
            max_layer,
        })
    }
}

/// Canonical in-memory HNSW graph representation for `selene-vector`.
#[derive(Debug)]
pub struct HnswGraph {
    pub(crate) nodes: Vec<HnswNode>,
    pub(crate) entry_point: Option<InternalIndex>,
    pub(crate) max_layer: u8,
    pub(crate) node_id_to_idx: HashMap<NodeId, InternalIndex>,
    pub(crate) dimensions: u16,
}

impl HnswGraph {
    /// Construct an empty graph with the configured dimensionality.
    #[must_use]
    pub fn empty(dimensions: u16) -> Self {
        Self {
            nodes: Vec::new(),
            entry_point: None,
            max_layer: 0,
            node_id_to_idx: HashMap::new(),
            dimensions,
        }
    }

    /// Construct an empty graph with storage reserved for `capacity` nodes.
    #[must_use]
    pub fn with_capacity(dimensions: u16, capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            entry_point: None,
            max_layer: 0,
            node_id_to_idx: HashMap::with_capacity(capacity),
            dimensions,
        }
    }

    /// Clone graph topology for one copy-on-write mutation.
    ///
    /// This is `O(N + E)` in node count plus neighbor-list entries. Dense
    /// vector payloads stay shared through their `Arc<[f32]>` handles.
    #[must_use]
    pub(crate) fn clone_for_mutation(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            entry_point: self.entry_point,
            max_layer: self.max_layer,
            node_id_to_idx: self.node_id_to_idx.clone(),
            dimensions: self.dimensions,
        }
    }

    /// Return the vector dimensionality as a slice-friendly `usize`.
    #[must_use]
    pub fn dimensions(&self) -> usize {
        usize::from(self.dimensions)
    }

    /// Return the number of indexed nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Return true when the graph contains no indexed nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Return the current search entry point, if any.
    #[must_use]
    pub fn entry_point(&self) -> Option<InternalIndex> {
        self.entry_point
    }

    /// Return the highest graph layer currently present.
    #[must_use]
    pub fn max_layer(&self) -> u8 {
        self.max_layer
    }

    /// Look up a node by source graph ID.
    #[must_use]
    pub fn node_by_id(&self, id: NodeId) -> Option<&HnswNode> {
        self.idx_for(id).and_then(|idx| self.node_by_idx(idx))
    }

    /// Look up a node by provider-local index.
    #[must_use]
    pub fn node_by_idx(&self, idx: InternalIndex) -> Option<&HnswNode> {
        let node = self.nodes.get(idx as usize)?;
        debug_assert!(
            self.assert_dim(&node.vector).is_ok(),
            "indexed vector dimensions must match graph dimensions"
        );
        Some(node)
    }

    /// Look up the provider-local index for a source graph node ID.
    #[must_use]
    pub fn idx_for(&self, id: NodeId) -> Option<InternalIndex> {
        self.node_id_to_idx.get(&id).copied()
    }

    /// Iterate over all indexed nodes in provider-local order.
    pub fn iter_nodes(&self) -> impl Iterator<Item = &HnswNode> {
        self.nodes.iter()
    }

    /// Return the neighbor list for `idx` at `layer`.
    #[must_use]
    pub fn iter_layer_neighbors(&self, idx: InternalIndex, layer: u8) -> Option<&[InternalIndex]> {
        let node = self.node_by_idx(idx)?;
        node.neighbors.get(layer as usize).map(Vec::as_slice)
    }

    fn assert_dim(&self, slice: &[f32]) -> Result<(), VectorError> {
        if slice.len() == self.dimensions() {
            Ok(())
        } else {
            Err(VectorError::DimensionMismatch {
                expected: self.dimensions(),
                observed: slice.len(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_dim_accepts_matching_slice() {
        let graph = HnswGraph::empty(3);
        graph.assert_dim(&[1.0, 2.0, 3.0]).unwrap();
    }

    #[test]
    fn assert_dim_rejects_mismatched_slice() {
        let graph = HnswGraph::empty(3);
        assert!(matches!(
            graph.assert_dim(&[1.0, 2.0]),
            Err(VectorError::DimensionMismatch {
                expected: 3,
                observed: 2
            })
        ));
    }
}
