//! Integration smoke tests for BRIEF-60 HNSW search.

use std::sync::Arc;

use roaring::RoaringBitmap;
use selene_core::NodeId;
use selene_vector::hnsw::search::search;
use selene_vector::hnsw::{HnswGraph, HnswParams, insert_node};
use selene_vector::{DistanceMetric, HnswConfig, VectorError};

type VectorFixture = Vec<(NodeId, Arc<[f32]>)>;
type PopulatedGraph = (HnswGraph, HnswParams, VectorFixture);

#[test]
fn search_empty_graph_returns_empty() {
    let params = params(4, DistanceMetric::Cosine);
    let graph = HnswGraph::empty(4);

    let results = search(&graph, &[1.0, 0.0, 0.0, 0.0], 10, 16, &params, None).unwrap();

    assert!(results.is_empty());
}

#[test]
fn search_k_zero_returns_empty() {
    let (graph, params, vectors) = populated_graph(5, 4, DistanceMetric::Cosine, 11);

    let results = search(&graph, &vectors[0].1, 0, 16, &params, None).unwrap();

    assert!(results.is_empty());
}

#[test]
fn search_dimension_mismatch_rejected() {
    let params = params(4, DistanceMetric::Cosine);
    let graph = HnswGraph::empty(4);

    let err = search(&graph, &[1.0, 2.0, 3.0], 1, 4, &params, None)
        .expect_err("dimension mismatch is rejected");

    assert!(matches!(
        err,
        VectorError::DimensionsLocked {
            expected: 4,
            observed: 3
        }
    ));
}

#[test]
fn search_self_recall_top1() {
    let (graph, params, vectors) = populated_graph(200, 32, DistanceMetric::Cosine, 42);

    for (node_id, vector) in vectors.iter().step_by(20).take(10) {
        let results = search(&graph, vector, 1, 200, &params, None).unwrap();
        assert_eq!(
            results.first().map(|(id, _)| *id),
            Some(*node_id),
            "self-search should rank {node_id} first"
        );
    }
}

#[test]
fn search_k_greater_than_n_truncates() {
    let (graph, params, vectors) = populated_graph(5, 4, DistanceMetric::Cosine, 7);

    let results = search(&graph, &vectors[0].1, 100, 100, &params, None).unwrap();

    assert_eq!(results.len(), 5);
}

#[test]
fn search_ef_below_k_widens_to_k() {
    let (graph, params, vectors) = populated_graph(30, 4, DistanceMetric::Cosine, 8);

    let narrow = search(&graph, &vectors[0].1, 10, 4, &params, None).unwrap();
    let at_k = search(&graph, &vectors[0].1, 10, 10, &params, None).unwrap();

    assert_eq!(narrow.len(), 10);
    assert_eq!(at_k.len(), 10);
}

#[test]
fn search_filter_restricts_results_to_bitmap() {
    let (graph, params, vectors) = populated_graph(30, 4, DistanceMetric::Cosine, 9);
    let mut filter = RoaringBitmap::new();
    for raw in (1..=30).filter(|raw| raw % 2 == 1) {
        filter.insert(raw);
    }

    let results = search(&graph, &vectors[0].1, 10, 30, &params, Some(&filter)).unwrap();

    assert!(!results.is_empty());
    assert!(results.iter().all(|(id, _)| id.get() % 2 == 1));
}

#[test]
fn search_filter_zero_hits_returns_empty() {
    let (graph, params, vectors) = populated_graph(30, 4, DistanceMetric::Cosine, 10);
    let filter = RoaringBitmap::new();

    let results = search(&graph, &vectors[0].1, 10, 30, &params, Some(&filter)).unwrap();

    assert!(results.is_empty());
}

#[test]
fn search_constructed_corpus_same_top1_across_metrics() {
    for metric in [
        DistanceMetric::Cosine,
        DistanceMetric::L2,
        DistanceMetric::Dot,
    ] {
        let params = params(2, metric);
        let mut graph = HnswGraph::empty(2);
        insert_node(&mut graph, NodeId::new(1), vector(&[1.0, 0.0]), 0, &params).unwrap();
        insert_node(&mut graph, NodeId::new(2), vector(&[-1.0, 0.0]), 0, &params).unwrap();
        insert_node(&mut graph, NodeId::new(3), vector(&[0.0, 1.0]), 0, &params).unwrap();

        let results = search(&graph, &[1.0, 0.0], 1, 3, &params, None).unwrap();

        assert_eq!(results.first().map(|(id, _)| *id), Some(NodeId::new(1)));
    }
}

#[test]
fn search_results_are_deterministic_across_equal_builds() {
    let (left_graph, left_params, left_vectors) =
        populated_graph(80, 8, DistanceMetric::Cosine, 99);
    let (right_graph, right_params, right_vectors) =
        populated_graph(80, 8, DistanceMetric::Cosine, 99);

    let left = search(&left_graph, &left_vectors[15].1, 12, 40, &left_params, None).unwrap();
    let right = search(
        &right_graph,
        &right_vectors[15].1,
        12,
        40,
        &right_params,
        None,
    )
    .unwrap();

    assert_eq!(left, right);
}

#[test]
fn search_filter_silently_excludes_nodeid_above_u32_max() {
    let params = params(2, DistanceMetric::Cosine);
    let mut graph = HnswGraph::empty(2);
    let raw_above_u32 = u64::from(u32::MAX) + 1;
    insert_node(
        &mut graph,
        NodeId::new(raw_above_u32),
        vector(&[1.0, 0.0]),
        0,
        &params,
    )
    .unwrap();
    let mut filter = RoaringBitmap::new();
    filter.insert(0);

    let results = search(&graph, &[1.0, 0.0], 1, 4, &params, Some(&filter)).unwrap();

    assert!(results.is_empty());
}

#[test]
fn search_filter_admission_still_expands_filtered_entry() {
    let params = params(2, DistanceMetric::L2);
    let mut graph = HnswGraph::empty(2);
    insert_node(&mut graph, NodeId::new(1), vector(&[10.0, 0.0]), 0, &params).unwrap();
    insert_node(&mut graph, NodeId::new(2), vector(&[0.0, 0.0]), 0, &params).unwrap();
    let mut filter = RoaringBitmap::new();
    filter.insert(2);

    let results = search(&graph, &[0.0, 0.0], 1, 4, &params, Some(&filter)).unwrap();

    assert_eq!(results.first().map(|(id, _)| *id), Some(NodeId::new(2)));
}

#[test]
fn search_rejects_nan_query() {
    let (graph, params, _) = populated_graph(5, 4, DistanceMetric::Cosine, 12);

    let err = search(&graph, &[1.0, f32::NAN, 0.0, 0.0], 1, 4, &params, None)
        .expect_err("NaN query rejected");

    assert!(matches!(
        err,
        VectorError::NonFiniteQueryComponent { index: 1, value } if value.is_nan()
    ));
}

#[test]
fn search_rejects_inf_query() {
    let (graph, params, _) = populated_graph(5, 4, DistanceMetric::Cosine, 13);

    let err = search(&graph, &[1.0, f32::INFINITY, 0.0, 0.0], 1, 4, &params, None)
        .expect_err("infinite query rejected");

    assert!(matches!(
        err,
        VectorError::NonFiniteQueryComponent { index: 1, value } if value.is_infinite()
    ));
}

fn populated_graph(count: u64, dim: usize, metric: DistanceMetric, seed: u64) -> PopulatedGraph {
    let params = params(dim, metric);
    let mut graph = HnswGraph::empty(dim as u16);
    let mut rng = fastrand::Rng::with_seed(seed);
    let mut vectors = Vec::with_capacity(count as usize);

    for raw in 1..=count {
        let components: Vec<f32> = (0..dim).map(|_| (rng.f32() * 2.0) - 1.0).collect();
        let vector = Arc::from(components.into_boxed_slice());
        insert_node(
            &mut graph,
            NodeId::new(raw),
            Arc::clone(&vector),
            0,
            &params,
        )
        .unwrap();
        vectors.push((NodeId::new(raw), vector));
    }

    (graph, params, vectors)
}

fn params(dim: usize, metric: DistanceMetric) -> HnswParams {
    let config = HnswConfig::with_params(dim, 16, 200, 50, metric).unwrap();
    HnswParams::from_config(&config)
}

fn vector(values: &[f32]) -> Arc<[f32]> {
    Arc::from(values)
}
