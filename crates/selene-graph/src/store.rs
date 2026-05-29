//! Structure-of-arrays node and edge stores per spec 03 section 3.1.

use roaring::RoaringBitmap;

use selene_core::{EdgeId, IStr, LabelSet, NodeId, PropertyMap};

use crate::chunked_vec::ChunkedVec;

/// Internal storage row index — the position of a node or edge in its store's
/// structure-of-arrays columns.
///
/// Distinct from the external [`NodeId`]/[`EdgeId`]: a `RowIndex` is dense,
/// reused after compaction (D22 / BRIEF-Item-4b), and **never persisted** — only
/// external ids reach the WAL, snapshot, or `Change` stream. Today the mapping
/// is the identity `RowIndex(r) <-> *Id(r + 1)`; the
/// [`SeleneGraph`](crate::SeleneGraph) `node_id_to_row`/`edge_id_to_row` maps and
/// the per-store `row_to_id` columns make it explicit so a later compaction epoch
/// can renumber rows while external ids stay stable. Keeping it a newtype lets
/// the compiler flag any site that still conflates a row with an external id.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RowIndex(u32);

impl RowIndex {
    /// Sentinel for "no row" (`u32::MAX`, never a valid dense row position).
    pub const TOMBSTONE: RowIndex = RowIndex(u32::MAX);

    /// Construct a `RowIndex` from a raw `u32` row position.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the raw `u32` row position.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Node columns plus liveness bitmap.
#[derive(Clone, Debug)]
pub struct NodeStore {
    /// Per-row node label sets.
    pub labels: ChunkedVec<LabelSet>,
    /// Per-row node property maps.
    pub properties: ChunkedVec<PropertyMap>,
    /// Per-row external node id (`RowIndex -> NodeId`). Dead / hole rows hold
    /// [`NodeId::TOMBSTONE`]. Parallel with [`labels`](Self::labels): the stable
    /// id is read here, never synthesized as `row + 1`, so compaction can
    /// renumber rows under stable ids.
    pub row_to_id: ChunkedVec<NodeId>,
    /// Alive row indexes.
    pub alive: RoaringBitmap,
}

impl NodeStore {
    /// Construct an empty node store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            labels: ChunkedVec::new(),
            properties: ChunkedVec::new(),
            row_to_id: ChunkedVec::new(),
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
///
/// Stored edges are directed by construction: every live row has exactly one
/// source node and one target node. Undirected query patterns are a matching
/// convenience and do not add an undirected storage bit.
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
    /// Per-row external edge id (`RowIndex -> EdgeId`). Dead / hole rows hold
    /// [`EdgeId::TOMBSTONE`]. Parallel with [`label`](Self::label): the stable id
    /// is read here, never synthesized as `row + 1`.
    pub row_to_id: ChunkedVec<EdgeId>,
    /// Alive row indexes.
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
            row_to_id: ChunkedVec::new(),
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
