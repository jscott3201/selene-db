//! Integration smoke tests for BRIEF-59 HNSW insertion.

use std::sync::Arc;

use selene_core::NodeId;
use selene_vector::hnsw::{HnswGraph, HnswParams, insert_node, random_layer};
use selene_vector::{DistanceMetric, HnswConfig, VectorError};

#[test]
fn insert_into_empty_sets_entry_point() {
    let mut graph = HnswGraph::empty(4);
    let idx = insert_node(
        &mut graph,
        NodeId::new(1),
        vector(&[1.0, 0.0, 0.0, 0.0]),
        2,
        &params(DistanceMetric::Cosine),
    )
    .expect("insert succeeds");

    assert_eq!(idx, 0);
    assert_eq!(graph.entry_point(), Some(0));
    assert_eq!(graph.max_layer(), 2);
    assert_eq!(graph.len(), 1);
}

#[test]
fn insert_n_nodes_all_reachable_via_id_map() {
    let mut rng = fastrand::Rng::with_seed(42);
    let mut graph = HnswGraph::empty(4);
    let params = params(DistanceMetric::Cosine);

    for raw in 1..=20 {
        insert_node(
            &mut graph,
            NodeId::new(raw),
            random_vector(&mut rng, 4),
            random_layer(&mut rng, params.level_factor),
            &params,
        )
        .expect("insert succeeds");
    }

    for raw in 1..=20 {
        assert!(graph.node_by_id(NodeId::new(raw)).is_some());
    }
    assert_eq!(graph.len(), 20);
}

#[test]
fn bidirectional_links_at_layer_0() {
    let graph = populated_graph(30, DistanceMetric::Cosine);

    for (idx, node) in graph.iter_nodes().enumerate() {
        for neighbor_idx in &node.neighbors[0] {
            let backlink = graph
                .iter_layer_neighbors(*neighbor_idx, 0)
                .expect("neighbor has layer 0");
            assert!(
                backlink.contains(&(idx as u32)),
                "node {idx} missing backlink from {neighbor_idx}"
            );
        }
    }
}

#[test]
fn pruning_respects_max_neighbors() {
    let graph = populated_graph(50, DistanceMetric::Cosine);

    for node in graph.iter_nodes() {
        assert!(node.neighbors[0].len() <= 8);
        for layer in 1..=node.max_layer {
            assert!(node.neighbors[layer as usize].len() <= 4);
        }
    }
}

#[test]
fn dimensions_locked_rejects_mismatch() {
    let mut graph = HnswGraph::empty(4);
    let err = insert_node(
        &mut graph,
        NodeId::new(1),
        vector(&[1.0, 2.0, 3.0]),
        0,
        &params(DistanceMetric::Cosine),
    )
    .expect_err("wrong dimension is rejected");

    assert!(matches!(
        err,
        VectorError::DimensionsLocked {
            expected: 4,
            observed: 3
        }
    ));
}

#[test]
fn insert_tombstone_rejected() {
    let mut graph = HnswGraph::empty(4);
    let err = insert_node(
        &mut graph,
        NodeId::TOMBSTONE,
        vector(&[1.0, 0.0, 0.0, 0.0]),
        0,
        &params(DistanceMetric::Cosine),
    )
    .expect_err("tombstone is rejected");

    assert!(matches!(
        err,
        VectorError::InvalidNodeId { node_id, .. } if node_id == NodeId::TOMBSTONE
    ));
}

#[test]
fn insert_duplicate_node_id_rejected() {
    let mut graph = HnswGraph::empty(4);
    let params = params(DistanceMetric::Cosine);
    insert_node(
        &mut graph,
        NodeId::new(1),
        vector(&[1.0, 0.0, 0.0, 0.0]),
        0,
        &params,
    )
    .expect("first insert succeeds");
    let err = insert_node(
        &mut graph,
        NodeId::new(1),
        vector(&[0.0, 1.0, 0.0, 0.0]),
        0,
        &params,
    )
    .expect_err("duplicate is rejected");

    assert!(matches!(
        err,
        VectorError::DuplicateNodeId { node_id } if node_id == NodeId::new(1)
    ));
}

#[test]
fn insert_rejects_nan_component() {
    let mut graph = HnswGraph::empty(3);
    let err = insert_node(
        &mut graph,
        NodeId::new(1),
        vector(&[1.0, f32::NAN, 3.0]),
        0,
        &HnswParams::from_config(&HnswConfig::new(3).unwrap()),
    )
    .expect_err("NaN is rejected");

    assert!(matches!(
        err,
        VectorError::NonFiniteVectorComponent { index: 1, value, .. } if value.is_nan()
    ));
}

#[test]
fn insert_rejects_inf_component() {
    let mut graph = HnswGraph::empty(3);
    let err = insert_node(
        &mut graph,
        NodeId::new(1),
        vector(&[1.0, f32::INFINITY, 3.0]),
        0,
        &HnswParams::from_config(&HnswConfig::new(3).unwrap()),
    )
    .expect_err("inf is rejected");

    assert!(matches!(
        err,
        VectorError::NonFiniteVectorComponent { index: 1, value, .. } if value.is_infinite()
    ));
}

#[test]
fn random_layer_distribution_default_factor() {
    let mut rng = fastrand::Rng::with_seed(42);
    let params = HnswParams::from_config(&HnswConfig::new(4).unwrap());
    let mut layer0 = 0;
    let mut layer3_plus = 0;

    for _ in 0..10_000 {
        let layer = random_layer(&mut rng, params.level_factor);
        if layer == 0 {
            layer0 += 1;
        }
        if layer >= 3 {
            layer3_plus += 1;
        }
    }

    assert!(layer0 >= 8_500, "layer0={layer0}");
    assert!(layer3_plus < 200, "layer3_plus={layer3_plus}");
}

#[test]
fn random_layer_seeded_is_deterministic() {
    let mut left = fastrand::Rng::with_seed(42);
    let mut right = fastrand::Rng::with_seed(42);
    let factor = params(DistanceMetric::Cosine).level_factor;

    let left_layers: Vec<_> = (0..32).map(|_| random_layer(&mut left, factor)).collect();
    let right_layers: Vec<_> = (0..32).map(|_| random_layer(&mut right, factor)).collect();

    assert_eq!(left_layers, right_layers);
}

#[test]
fn l2_build_selection_is_not_cosine_hardcoded() {
    let mut graph = HnswGraph::empty(2);
    let params =
        HnswParams::from_config(&HnswConfig::with_params(2, 2, 8, 8, DistanceMetric::L2).unwrap());

    insert_node(&mut graph, NodeId::new(1), vector(&[1.0, 0.0]), 0, &params).unwrap();
    insert_node(
        &mut graph,
        NodeId::new(2),
        vector(&[100.0, 0.0]),
        0,
        &params,
    )
    .unwrap();
    insert_node(&mut graph, NodeId::new(3), vector(&[90.0, 0.0]), 0, &params).unwrap();

    let new_node = graph.node_by_idx(2).expect("new node is present");
    assert_eq!(
        new_node.neighbors[0].first().copied(),
        Some(1),
        "L2 should prefer the 100.0 vector over the lower-index colinear vector"
    );
}

fn populated_graph(count: u64, metric: DistanceMetric) -> HnswGraph {
    let mut rng = fastrand::Rng::with_seed(42);
    let mut graph = HnswGraph::empty(4);
    let params = params(metric);
    for raw in 1..=count {
        insert_node(
            &mut graph,
            NodeId::new(raw),
            random_vector(&mut rng, 4),
            random_layer(&mut rng, params.level_factor),
            &params,
        )
        .expect("insert succeeds");
    }
    graph
}

fn params(metric: DistanceMetric) -> HnswParams {
    HnswParams::from_config(&HnswConfig::with_params(4, 4, 16, 8, metric).unwrap())
}

fn random_vector(rng: &mut fastrand::Rng, dim: usize) -> Arc<[f32]> {
    (0..dim)
        .map(|_| (rng.f32() * 2.0) - 1.0)
        .collect::<Vec<_>>()
        .into()
}

fn vector(values: &[f32]) -> Arc<[f32]> {
    values.to_vec().into()
}
