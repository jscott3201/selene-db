//! Adjacency entries per spec 03 section 3.1.

use smallvec::SmallVec;

use selene_core::{EdgeId, IStr, NodeId};

/// One edge recorded in a node's incoming or outgoing adjacency list.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
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

    /// Iterate adjacent edges with exactly `label`.
    ///
    /// Entries are sorted by `(label, neighbor, edge_id)`, so this uses a
    /// bounded label range rather than scanning unrelated edge labels.
    pub fn iter_label<'a>(&'a self, label: &IStr) -> impl Iterator<Item = &'a AdjacencyEdge> {
        let range = self.label_range(label);
        self.edges[range].iter()
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

    fn label_range(&self, label: &IStr) -> std::ops::Range<usize> {
        let start = self
            .edges
            .partition_point(|edge| edge.label.as_str() < label.as_str());
        let len = self.edges[start..].partition_point(|edge| edge.label.as_str() == label.as_str());
        start..start + len
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
    (edge.label.clone(), edge.neighbor, edge.edge_id)
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
        entry.add(c.clone());
        entry.add(a.clone());
        entry.add(b.clone());
        assert_eq!(entry.iter().cloned().collect::<Vec<_>>(), vec![b, a, c]);
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
        entry.add(e.clone());
        assert_eq!(entry.remove(EdgeId::new(1)), Some(e));
        assert!(entry.is_empty());
    }

    #[test]
    fn remove_returns_none_when_absent() {
        let mut entry = AdjacencyEntry::new();
        assert_eq!(entry.remove(EdgeId::new(1)), None);
    }

    #[test]
    fn iter_label_returns_only_matching_label_range() {
        let mut entry = AdjacencyEntry::new();
        let a1 = edge("adj.a", 3, 1);
        let a2 = edge("adj.a", 4, 2);
        let b = edge("adj.b", 5, 3);
        let c = edge("adj.c", 6, 4);
        entry.add(c);
        entry.add(a2.clone());
        entry.add(b);
        entry.add(a1.clone());

        assert_eq!(
            entry
                .iter_label(&label("adj.a"))
                .cloned()
                .collect::<Vec<_>>(),
            vec![a1, a2],
        );
    }

    #[test]
    fn iter_label_returns_empty_for_absent_labels() {
        let mut entry = AdjacencyEntry::new();
        entry.add(edge("adj.a", 2, 1));
        entry.add(edge("adj.c", 3, 2));

        assert_eq!(entry.iter_label(&label("adj.b")).count(), 0);
    }

    #[test]
    fn parallel_edges_sort_by_edge_id() {
        let mut entry = AdjacencyEntry::new();
        let first = edge("adj.parallel", 2, 1);
        let second = edge("adj.parallel", 2, 2);
        entry.add(second.clone());
        entry.add(first.clone());
        assert_eq!(
            entry.iter().cloned().collect::<Vec<_>>(),
            vec![first, second]
        );
    }
}
