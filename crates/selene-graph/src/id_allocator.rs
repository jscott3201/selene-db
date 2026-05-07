//! Per-graph ID allocator per D11.
//!
//! The allocator is owned by `SharedGraph`, not by individual transactions.
//! Advancing a counter is permanent even when the transaction later rolls back,
//! which preserves spec 02 section 4's no-reuse identity rule.

use selene_core::{EdgeId, NodeId};

use crate::graph::GraphMeta;

/// Per-graph node and edge ID allocator.
#[derive(Clone, Debug)]
pub struct IdAllocator {
    next_node_id: u64,
    next_edge_id: u64,
}

impl IdAllocator {
    /// Construct an allocator at the v1.0 initial checkpoint.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_node_id: 1,
            next_edge_id: 1,
        }
    }

    /// Restore allocator counters from graph metadata.
    #[must_use]
    pub const fn from_meta(meta: &GraphMeta) -> Self {
        Self {
            next_node_id: meta.next_node_id,
            next_edge_id: meta.next_edge_id,
        }
    }

    /// Allocate a node ID and advance the permanent high-water mark.
    #[must_use]
    pub fn allocate_node(&mut self) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id = self
            .next_node_id
            .checked_add(1)
            .expect("node id allocator exhausted");
        NodeId::new(id)
    }

    /// Allocate an edge ID and advance the permanent high-water mark.
    #[must_use]
    pub fn allocate_edge(&mut self) -> EdgeId {
        let id = self.next_edge_id;
        self.next_edge_id = self
            .next_edge_id
            .checked_add(1)
            .expect("edge id allocator exhausted");
        EdgeId::new(id)
    }

    /// Return the next node ID without allocating it.
    #[must_use]
    pub const fn peek_next_node(&self) -> u64 {
        self.next_node_id
    }

    /// Return the next edge ID without allocating it.
    #[must_use]
    pub const fn peek_next_edge(&self) -> u64 {
        self.next_edge_id
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use selene_core::GraphId;

    #[test]
    fn allocate_node_advances_counter() {
        let mut allocator = IdAllocator::new();
        assert_eq!(allocator.allocate_node(), NodeId::new(1));
        assert_eq!(allocator.peek_next_node(), 2);
    }

    #[test]
    fn allocate_edge_advances_counter() {
        let mut allocator = IdAllocator::new();
        assert_eq!(allocator.allocate_edge(), EdgeId::new(1));
        assert_eq!(allocator.peek_next_edge(), 2);
    }

    #[test]
    fn from_meta_restores_counters() {
        let meta = GraphMeta {
            graph_id: GraphId::new(1),
            generation: 7,
            next_node_id: 42,
            next_edge_id: 99,
        };
        let allocator = IdAllocator::from_meta(&meta);
        assert_eq!(allocator.peek_next_node(), 42);
        assert_eq!(allocator.peek_next_edge(), 99);
    }
}
