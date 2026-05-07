//! Adjacency entries per spec 03 section 3.1.

use smallvec::SmallVec;

use selene_core::{EdgeId, IStr, NodeId};

/// One edge recorded in a node's incoming or outgoing adjacency list.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AdjacencyEdge {
    /// Edge label.
    pub label: IStr,
    /// Opposite endpoint reached through this edge.
    pub neighbor: NodeId,
    /// Stable edge ID.
    pub edge_id: EdgeId,
}

/// Sorted adjacency list for one node.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AdjacencyEntry {
    /// Edges sorted by `(label, neighbor, edge_id)`.
    pub edges: SmallVec<[AdjacencyEdge; 4]>,
}

impl AdjacencyEntry {
    /// Construct an empty adjacency entry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            edges: SmallVec::new(),
        }
    }

    /// Number of adjacent edges.
    #[must_use]
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Return true when no adjacent edges are recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Iterate adjacent edges in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = &AdjacencyEdge> {
        self.edges.iter()
    }

    /// Insert `edge` while maintaining sorted order.
    pub fn add(&mut self, edge: AdjacencyEdge) {
        let key = adjacency_key(&edge);
        let index = self
            .edges
            .binary_search_by_key(&key, adjacency_key)
            .unwrap_or_else(|index| index);
        self.edges.insert(index, edge);
    }

    /// Remove the edge with `edge_id`, returning it when present.
    pub fn remove(&mut self, edge_id: EdgeId) -> Option<AdjacencyEdge> {
        self.edges
            .iter()
            .position(|edge| edge.edge_id == edge_id)
            .map(|index| self.edges.remove(index))
    }

    #[cfg(test)]
    fn spilled(&self) -> bool {
        self.edges.spilled()
    }
}

impl Default for AdjacencyEntry {
    fn default() -> Self {
        Self::new()
    }
}

fn adjacency_key(edge: &AdjacencyEdge) -> (IStr, NodeId, EdgeId) {
    (edge.label, edge.neighbor, edge.edge_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use selene_core::intern;

    fn label(name: &str) -> IStr {
        intern(name).unwrap()
    }

    fn edge(label_name: &str, neighbor: u64, edge_id: u64) -> AdjacencyEdge {
        AdjacencyEdge {
            label: label(label_name),
            neighbor: NodeId::new(neighbor),
            edge_id: EdgeId::new(edge_id),
        }
    }

    #[test]
    fn add_inserts_in_sorted_order() {
        let mut entry = AdjacencyEntry::new();
        let a = edge("adj.a", 2, 2);
        let b = edge("adj.a", 2, 1);
        let c = edge("adj.b", 1, 3);
        entry.add(c);
        entry.add(a);
        entry.add(b);
        assert_eq!(entry.iter().copied().collect::<Vec<_>>(), vec![b, a, c]);
    }

    #[test]
    fn add_handles_inline_path_under_4_edges() {
        let mut entry = AdjacencyEntry::new();
        for id in 1..=4 {
            entry.add(edge("adj.inline", id, id));
        }
        assert_eq!(entry.len(), 4);
        assert!(!entry.spilled());
    }

    #[test]
    fn add_spills_after_4_edges() {
        let mut entry = AdjacencyEntry::new();
        for id in 1..=5 {
            entry.add(edge("adj.spill", id, id));
        }
        assert!(entry.spilled());
    }

    #[test]
    fn remove_returns_removed_edge() {
        let mut entry = AdjacencyEntry::new();
        let e = edge("adj.remove", 2, 1);
        entry.add(e);
        assert_eq!(entry.remove(EdgeId::new(1)), Some(e));
        assert!(entry.is_empty());
    }

    #[test]
    fn remove_returns_none_when_absent() {
        let mut entry = AdjacencyEntry::new();
        assert_eq!(entry.remove(EdgeId::new(1)), None);
    }

    #[test]
    fn parallel_edges_sort_by_edge_id() {
        let mut entry = AdjacencyEntry::new();
        let first = edge("adj.parallel", 2, 1);
        let second = edge("adj.parallel", 2, 2);
        entry.add(second);
        entry.add(first);
        assert_eq!(
            entry.iter().copied().collect::<Vec<_>>(),
            vec![first, second]
        );
    }
}
