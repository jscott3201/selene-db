//! Shared helpers for graph/vector retrieval benchmark fixtures.

use std::collections::HashMap;

use selene_algorithms::{
    GraphProjection, PageRankConfig, Parallelism, ProjectionConfig, pagerank, wcc,
};
use selene_core::{GraphId, IStr, NodeId, VectorValue, intern};
use selene_graph::SeleneGraph;
use selene_testing::BenchProfile;

pub(super) const DIMENSION: usize = 128;
pub(super) const FACTS_PER_TOPIC: usize = 8;
pub(super) const RESULT_K: usize = FACTS_PER_TOPIC;
pub(super) const SEED_K: usize = 4;
pub(super) const WIDE_SEED_K: usize = 16;
pub(super) const ORACLE_SEED_K: usize = 64;
pub(super) const SEARCH_WIDTH: usize = 32;
pub(super) const PAGERANK_WEIGHT: f64 = 0.05;

pub(super) fn graph_id_for_scale(requested_scale: usize) -> GraphId {
    GraphId::new(91_000 + requested_scale as u64)
}

pub(super) fn pagerank_scores(
    graph: &SeleneGraph,
    label: &IStr,
    support_edge: &IStr,
) -> HashMap<NodeId, f64> {
    let projection = GraphProjection::build(
        graph,
        &ProjectionConfig {
            name: "memory_retrieval".to_owned(),
            node_labels: vec![label.clone()],
            edge_labels: vec![support_edge.clone()],
            weight_property: None,
        },
        None,
    )
    .expect("bench projection builds");
    let scores = pagerank(
        &projection,
        PageRankConfig {
            damping: 0.85,
            max_iter: 32,
            tolerance: 1e-6,
            parallelism: Parallelism::Sequential,
        },
    );
    let max = scores.iter().map(|(_, score)| *score).fold(0.0, f64::max);
    scores
        .into_iter()
        .map(|(node, score)| (node, if max > 0.0 { score / max } else { 0.0 }))
        .collect()
}

pub(super) fn component_candidates(
    graph: &SeleneGraph,
    label: &IStr,
    support_edge: &IStr,
    superseded_by_edge: &IStr,
) -> (HashMap<NodeId, u64>, HashMap<u64, Vec<NodeId>>) {
    let projection = GraphProjection::build(
        graph,
        &ProjectionConfig {
            name: "memory_components".to_owned(),
            node_labels: vec![label.clone()],
            edge_labels: vec![support_edge.clone(), superseded_by_edge.clone()],
            weight_property: None,
        },
        None,
    )
    .expect("bench component projection builds");
    let mut by_node = HashMap::new();
    let mut by_component: HashMap<u64, Vec<NodeId>> = HashMap::new();
    for (node, component) in wcc(&projection) {
        by_node.insert(node, component);
        by_component.entry(component).or_default().push(node);
    }
    (by_node, by_component)
}

pub(super) fn topic_count(scale: usize) -> usize {
    (scale / (FACTS_PER_TOPIC * 4)).clamp(4, 64)
}

pub(super) fn duplicates_per_fact(scale: usize, topics: usize) -> usize {
    (scale / (topics * FACTS_PER_TOPIC)).max(2)
}

pub(super) fn current_replacement(nodes: &[NodeId], duplicate: usize) -> NodeId {
    let replacement = if duplicate.is_multiple_of(2) {
        duplicate
    } else {
        duplicate.saturating_sub(1)
    };
    nodes[replacement]
}

pub(super) fn vector_scales() -> Vec<usize> {
    std::env::var("SELENE_VECTOR_BENCH_SCALES")
        .ok()
        .and_then(parse_scales)
        .unwrap_or_else(|| BenchProfile::from_env().scales().to_vec())
}

fn parse_scales(raw: String) -> Option<Vec<usize>> {
    let mut scales: Vec<_> = raw
        .split(',')
        .filter_map(|part| part.trim().parse::<usize>().ok())
        .filter(|scale| *scale > 0)
        .collect();
    scales.sort_unstable();
    scales.dedup();
    (!scales.is_empty()).then_some(scales)
}

pub(super) fn memory_vector(
    topic: usize,
    fact: usize,
    duplicate: usize,
    shift: f32,
) -> VectorValue {
    let primary = topic % DIMENSION;
    let secondary = topic.wrapping_mul(5).wrapping_add(3) % DIMENSION;
    let fact_dim = topic.wrapping_mul(11).wrapping_add(fact * 17 + 7) % DIMENSION;
    let components: Vec<f32> = (0..DIMENSION)
        .map(|dim| {
            let base = (((topic + 3) * (dim + 11)) % 17) as f32 / 200.0;
            let topic_signal = if dim == primary { 1.0 } else { 0.0 };
            let secondary_signal = if dim == secondary { 0.25 } else { 0.0 };
            let fact_signal = if dim == fact_dim {
                fact as f32 * 0.055
            } else {
                0.0
            };
            let duplicate_noise =
                ((duplicate * (dim + 13) + fact * 31 + topic * 7) % 29) as f32 / 100_000.0;
            base + topic_signal + secondary_signal + fact_signal + duplicate_noise + shift
        })
        .collect();
    VectorValue::new(components).expect("bench vector is valid")
}

pub(super) fn basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}

pub(super) fn istr(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}
