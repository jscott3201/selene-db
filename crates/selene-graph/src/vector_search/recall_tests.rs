use std::collections::BTreeSet;

use selene_core::{
    CancellationChecker, GraphId, IStr, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap,
    Value, VectorMetric, VectorValue, intern,
};

use super::ApproximateVectorSearchOptions;
use crate::{SharedGraph, VectorIndexKind, VectorNodeSearchHit};

const K: usize = 8;
const EF_SEARCH: usize = 128;
const DISTANCE_TIE_EPSILON: f64 = 1e-9;

#[test]
fn hnsw_recall_handles_clustered_high_dimensional_cosine_vectors() {
    let profile = RecallProfile::build(
        9801,
        "vector.ann.recall.clustered.cosine",
        VectorIndexKind::HnswCosine,
        VectorMetric::Cosine,
        16,
        clustered_cosine_corpus(8, 24, 16),
        (0..8)
            .map(|cluster| clustered_cosine_vector(cluster, 24, 12, 16, 0.0003))
            .collect(),
    );

    assert_recall_at_least(&profile, 95);
    assert_distance_quality_at_least(&profile, 100);
}

#[test]
fn hnsw_recall_handles_negative_inner_product_vectors() {
    let profile = RecallProfile::build(
        9802,
        "vector.ann.recall.mips",
        VectorIndexKind::HnswNegativeInnerProduct,
        VectorMetric::NegativeInnerProduct,
        12,
        mips_corpus(256, 12),
        vec![
            mips_query(15, 12),
            mips_query(63, 12),
            mips_query(127, 12),
            mips_query(211, 12),
        ],
    );

    assert_recall_at_least(&profile, 90);
}

#[test]
fn hnsw_recall_survives_update_delete_churn() {
    let label = intern("vector.ann.recall.churn").unwrap();
    let property = intern("embedding").unwrap();
    let shared = SharedGraph::new(GraphId::new(9803));
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for row in 0..192 {
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    props(&property, line_vector(row, 6)),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    shared
        .create_vector_index(
            label.clone(),
            property.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            6,
        )
        .unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for row in (0..192).step_by(9) {
            let node_id = NodeId::new(row + 1);
            mutator
                .update_node(
                    node_id,
                    LabelDiff::new([], []).unwrap(),
                    PropertyDiff::new([(property.clone(), line_vector(row + 384, 6))], []).unwrap(),
                )
                .unwrap();
        }
        for row in (5..192).step_by(11) {
            mutator.delete_node(NodeId::new(row + 1)).unwrap();
        }
        txn.commit().unwrap();
    }

    let queries = [4, 41, 88, 151]
        .into_iter()
        .map(|row| line_query(row, 6))
        .collect();
    let profile = RecallProfile::from_graph(
        shared,
        label,
        property,
        VectorMetric::SquaredEuclidean,
        queries,
    );

    assert_recall_at_least(&profile, 85);
    for query in &profile.queries {
        let approximate = profile.approximate(query);
        assert_unique_hits(&approximate);
    }
}

#[test]
fn hnsw_recall_quality_accepts_duplicate_distance_ties() {
    let profile = RecallProfile::build(
        9804,
        "vector.ann.recall.tie.heavy.cosine",
        VectorIndexKind::HnswCosine,
        VectorMetric::Cosine,
        16,
        duplicate_cosine_corpus(8, 32, 16),
        (0..8)
            .map(|cluster| duplicate_cosine_vector(cluster, 16))
            .collect(),
    );

    assert_distance_quality_at_least(&profile, 100);
    for query in &profile.queries {
        let approximate = profile.approximate(query);
        assert_unique_hits(&approximate);
    }
}

struct RecallProfile {
    graph: SharedGraph,
    label: IStr,
    property: IStr,
    metric: VectorMetric,
    queries: Vec<VectorValue>,
    exact: Vec<Vec<VectorNodeSearchHit>>,
}

impl RecallProfile {
    fn build(
        graph_id: u64,
        label_name: &'static str,
        kind: VectorIndexKind,
        metric: VectorMetric,
        dimension: u32,
        corpus: Vec<VectorValue>,
        queries: Vec<VectorValue>,
    ) -> Self {
        let label = intern(label_name).unwrap();
        let property = intern("embedding").unwrap();
        let graph = SharedGraph::new(GraphId::new(graph_id));
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            for vector in corpus {
                mutator
                    .create_node(
                        LabelSet::single(label.clone()),
                        props(&property, Value::Vector(vector)),
                    )
                    .unwrap();
            }
            txn.commit().unwrap();
        }
        graph
            .create_vector_index(label.clone(), property.clone(), kind, dimension)
            .unwrap();
        Self::from_graph(graph, label, property, metric, queries)
    }

    fn from_graph(
        graph: SharedGraph,
        label: IStr,
        property: IStr,
        metric: VectorMetric,
        queries: Vec<VectorValue>,
    ) -> Self {
        let exact = queries
            .iter()
            .map(|query| {
                graph
                    .exact_vector_search_nodes(&label, &property, query, metric, K)
                    .unwrap()
            })
            .collect();
        Self {
            graph,
            label,
            property,
            metric,
            queries,
            exact,
        }
    }

    fn approximate(&self, query: &VectorValue) -> Vec<VectorNodeSearchHit> {
        self.graph
            .approximate_vector_search_nodes_checked(
                &self.label,
                &self.property,
                query,
                ApproximateVectorSearchOptions::new(self.metric, K, EF_SEARCH),
                CancellationChecker::disabled(),
            )
            .unwrap()
    }
}

fn assert_distance_quality_at_least(profile: &RecallProfile, floor_percent: usize) {
    let mut quality = 0usize;
    let mut expected = 0usize;
    for (query, exact) in profile.queries.iter().zip(&profile.exact) {
        let approximate = profile.approximate(query);
        expected += exact.len();
        quality += distance_quality_count(exact, &approximate);
    }

    assert!(
        quality * 100 >= expected * floor_percent,
        "HNSW distance quality {quality}/{expected} fell below {floor_percent}%"
    );
}

fn assert_recall_at_least(profile: &RecallProfile, floor_percent: usize) {
    let mut overlap = 0usize;
    let mut expected = 0usize;
    for (query, exact) in profile.queries.iter().zip(&profile.exact) {
        let approximate = profile.approximate(query);
        expected += exact.len();
        overlap += overlap_count(exact, &approximate);
    }

    assert!(
        overlap * 100 >= expected * floor_percent,
        "HNSW recall {overlap}/{expected} fell below {floor_percent}%"
    );
}

fn assert_unique_hits(hits: &[VectorNodeSearchHit]) {
    let mut seen = BTreeSet::new();
    for hit in hits {
        assert!(seen.insert(hit.node_id), "duplicate ANN hit: {:?}", hit);
    }
}

fn distance_quality_count(
    exact: &[VectorNodeSearchHit],
    approximate: &[VectorNodeSearchHit],
) -> usize {
    let Some(threshold) = exact.last().map(|hit| hit.distance + DISTANCE_TIE_EPSILON) else {
        return 0;
    };
    approximate
        .iter()
        .take(exact.len())
        .filter(|hit| hit.distance <= threshold)
        .count()
}

fn overlap_count(exact: &[VectorNodeSearchHit], approximate: &[VectorNodeSearchHit]) -> usize {
    exact
        .iter()
        .filter(|expected| {
            approximate
                .iter()
                .any(|hit| hit.node_id == expected.node_id)
        })
        .count()
}

fn props(property: &IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(property.clone(), value)]).unwrap()
}

fn clustered_cosine_corpus(
    clusters: usize,
    per_cluster: usize,
    dimension: usize,
) -> Vec<VectorValue> {
    (0..clusters)
        .flat_map(|cluster| {
            (0..per_cluster).map(move |offset| {
                clustered_cosine_vector(cluster, per_cluster, offset, dimension, 0.0)
            })
        })
        .collect()
}

fn clustered_cosine_vector(
    cluster: usize,
    per_cluster: usize,
    offset: usize,
    dimension: usize,
    query_shift: f32,
) -> VectorValue {
    let center = cluster % dimension;
    let second = cluster.wrapping_mul(5).wrapping_add(3) % dimension;
    let spread = offset as f32 - (per_cluster as f32 / 2.0);
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let base = (((cluster + 3) * (dim + 11)) % 17) as f32 / 200.0;
            let primary = if dim == center { 1.0 } else { 0.0 };
            let secondary = if dim == second { 0.25 } else { 0.0 };
            base + primary + secondary + spread * 0.0002 + query_shift
        })
        .collect();
    VectorValue::new(components).unwrap()
}

fn duplicate_cosine_corpus(
    clusters: usize,
    per_cluster: usize,
    dimension: usize,
) -> Vec<VectorValue> {
    (0..clusters)
        .flat_map(|cluster| {
            let vector = duplicate_cosine_vector(cluster, dimension);
            std::iter::repeat_n(vector, per_cluster)
        })
        .collect()
}

fn duplicate_cosine_vector(cluster: usize, dimension: usize) -> VectorValue {
    let center = cluster % dimension;
    let second = cluster.wrapping_mul(5).wrapping_add(3) % dimension;
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            if dim == center {
                1.0
            } else if dim == second {
                0.25
            } else {
                0.0
            }
        })
        .collect();
    VectorValue::new(components).unwrap()
}

fn mips_corpus(scale: usize, dimension: usize) -> Vec<VectorValue> {
    (0..scale)
        .map(|seed| {
            let components: Vec<f32> = (0..dimension)
                .map(|dim| {
                    let trend = seed as f32 / scale as f32;
                    let local = ((seed * (dim + 13) + dim * 29) % 101) as f32 / 5_000.0;
                    trend * (1.0 + dim as f32 / dimension as f32) + local + 0.01
                })
                .collect();
            VectorValue::new(components).unwrap()
        })
        .collect()
}

fn mips_query(seed: usize, dimension: usize) -> VectorValue {
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let weight = 1.0 + dim as f32 / dimension as f32;
            let tilt = ((seed + dim * 7) % 23) as f32 / 1_000.0;
            weight + tilt
        })
        .collect();
    VectorValue::new(components).unwrap()
}

fn line_vector(row: u64, dimension: usize) -> Value {
    Value::Vector(line_query(row, dimension))
}

fn line_query(row: u64, dimension: usize) -> VectorValue {
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            if dim == 0 {
                row as f32
            } else {
                ((row as usize * (dim + 7) + dim * 31) % 997) as f32 / 1_000.0
            }
        })
        .collect();
    VectorValue::new(components).unwrap()
}
