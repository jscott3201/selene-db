//! Structure-of-arrays node and edge stores per spec 03 section 3.1.

use roaring::RoaringBitmap;

use selene_core::{EdgeId, IStr, LabelSet, NodeId, PropertyMap};

use crate::chunked_vec::ChunkedVec;

/// Node columns plus liveness bitmap.
#[derive(Clone, Debug)]
pub struct NodeStore {
    /// Per-row node label sets.
    pub labels: ChunkedVec<LabelSet>,
    /// Per-row node property maps.
    pub properties: ChunkedVec<PropertyMap>,
    /// Alive row indexes. Row `r` corresponds to `NodeId(r + 1)`.
    pub alive: RoaringBitmap,
}

impl NodeStore {
    /// Construct an empty node store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            labels: ChunkedVec::new(),
            properties: ChunkedVec::new(),
            alive: RoaringBitmap::new(),
        }
    }

    /// Number of allocated node rows, including dead holes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// Return true when no node rows exist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return true when `index` is alive.
    #[must_use]
    pub fn is_alive(&self, index: u32) -> bool {
        self.alive.contains(index)
    }
}

impl Default for NodeStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Edge columns plus liveness bitmap.
#[derive(Clone, Debug)]
pub struct EdgeStore {
    /// Per-row edge label.
    pub label: ChunkedVec<IStr>,
    /// Per-row edge source node.
    pub source: ChunkedVec<NodeId>,
    /// Per-row edge target node.
    pub target: ChunkedVec<NodeId>,
    /// Per-row edge property maps.
    pub properties: ChunkedVec<PropertyMap>,
    /// Alive row indexes. Row `r` corresponds to `EdgeId(r + 1)`.
    pub alive: RoaringBitmap,
}

impl EdgeStore {
    /// Construct an empty edge store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            label: ChunkedVec::new(),
            source: ChunkedVec::new(),
            target: ChunkedVec::new(),
            properties: ChunkedVec::new(),
            alive: RoaringBitmap::new(),
        }
    }

    /// Number of allocated edge rows, including dead holes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.label.len()
    }

    /// Return true when no edge rows exist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return true when `index` is alive.
    #[must_use]
    pub fn is_alive(&self, index: u32) -> bool {
        self.alive.contains(index)
    }
}

impl Default for EdgeStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a node ID to its store row index.
#[must_use]
pub fn node_row_index(id: NodeId) -> Option<u32> {
    id.get()
        .checked_sub(1)
        .and_then(|raw| u32::try_from(raw).ok())
}

/// Convert an edge ID to its store row index.
#[must_use]
pub fn edge_row_index(id: EdgeId) -> Option<u32> {
    id.get()
        .checked_sub(1)
        .and_then(|raw| u32::try_from(raw).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use selene_core::intern;

    #[test]
    fn node_store_new_is_empty() {
        let store = NodeStore::new();
        assert_eq!(store.len(), 0);
        assert!(store.alive.is_empty());
    }

    #[test]
    fn edge_store_new_is_empty() {
        let store = EdgeStore::new();
        assert_eq!(store.len(), 0);
        assert!(store.alive.is_empty());
    }

    #[test]
    fn is_alive_after_simulated_insert() {
        let mut store = NodeStore::new();
        store
            .labels
            .push(LabelSet::single(intern("store.node").unwrap()));
        store.properties.push(PropertyMap::new());
        store.alive.insert(0);
        assert!(store.is_alive(0));
        assert!(!store.is_alive(1));
    }

    #[test]
    fn row_index_maps_id_minus_one() {
        assert_eq!(node_row_index(NodeId::new(1)), Some(0));
        assert_eq!(node_row_index(NodeId::new(42)), Some(41));
        assert_eq!(node_row_index(NodeId::TOMBSTONE), None);
        assert_eq!(edge_row_index(EdgeId::new(1)), Some(0));
    }
}
