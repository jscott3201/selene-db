//! Integration smoke tests for the BRIEF-58 HNSW graph shape.

use std::sync::Arc;

use selene_core::NodeId;
use selene_vector::{HnswConfig, HnswGraph, HnswNode, HnswProvider, VectorError};

#[test]
fn empty_graph_invariants() {
    let graph = HnswGraph::empty(384);

    assert_eq!(graph.dimensions(), 384);
    assert_eq!(graph.len(), 0);
    assert!(graph.is_empty());
    assert_eq!(graph.entry_point(), None);
    assert_eq!(graph.max_layer(), 0);
    assert!(graph.node_by_id(NodeId::new(1)).is_none());
    assert!(graph.node_by_idx(0).is_none());
    assert!(graph.idx_for(NodeId::new(1)).is_none());
    assert_eq!(graph.iter_nodes().count(), 0);
    assert!(graph.iter_layer_neighbors(0, 0).is_none());
}

#[test]
fn with_capacity_does_not_change_observable_state() {
    let graph = HnswGraph::with_capacity(384, 1_000);

    assert_eq!(graph.dimensions(), 384);
    assert_eq!(graph.len(), 0);
    assert!(graph.is_empty());
    assert_eq!(graph.entry_point(), None);
    assert_eq!(graph.max_layer(), 0);
}

#[test]
fn hnsw_node_constructor_refuses_tombstone() {
    let err = HnswNode::new(NodeId::TOMBSTONE, vector(4), 0)
        .expect_err("tombstone IDs cannot be indexed");

    assert!(matches!(
        err,
        VectorError::InvalidNodeId { node_id, reason }
            if node_id == NodeId::TOMBSTONE
                && reason.contains("NodeId::TOMBSTONE cannot be added")
    ));
}

#[test]
fn hnsw_node_constructor_allocates_per_layer_neighbor_lists() {
    let node = HnswNode::new(NodeId::new(1), vector(4), 3).expect("node id is valid");

    assert_eq!(node.node_id, NodeId::new(1));
    assert_eq!(node.vector.len(), 4);
    assert_eq!(node.max_layer, 3);
    assert_eq!(node.neighbors.len(), 4);
    assert!(node.neighbors.iter().all(Vec::is_empty));
}

#[test]
fn provider_snapshot_starts_empty() {
    let config = HnswConfig::new(384).expect("config is valid");
    let provider = HnswProvider::new(config).expect("provider config is valid");
    let snapshot = provider.snapshot();

    assert_eq!(snapshot.dimensions(), 384);
    assert!(snapshot.is_empty());
    assert!(
        Arc::strong_count(&snapshot) >= 2,
        "ArcSwap retains one Arc while caller holds the loaded snapshot"
    );
}

fn vector(dim: usize) -> Arc<[f32]> {
    vec![1.0; dim].into()
}
