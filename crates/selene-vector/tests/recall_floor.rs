//! Deterministic recall floor for BRIEF-65 neighbor-selection tuning.

use std::collections::HashSet;
use std::sync::Arc;

use selene_core::{Change, NodeId, intern};
use selene_graph::IndexProvider;
use selene_vector::distance::distance;
use selene_vector::{
    DistanceMetric, HnswConfig, HnswParams, HnswProvider, NeighborSelectionConfig, VectorOp,
    VectorUpsertPayloadV1, random_layer,
};

const VECTOR_SEED: u64 = 0xB65E_0001_u64;
const QUERY_SEED: u64 = 0xB65E_0002_u64;
const LAYER_SEED: u64 = 0xB65E_0003_u64;
const RECALL_FLOOR: f32 = 0.85;

#[test]
fn recall_at_10_meets_floor() {
    if cfg!(feature = "simd-simsimd") {
        return;
    }

    let recall = mean_recall(NeighborSelectionConfig::default(), 50);

    assert!(
        recall >= RECALL_FLOOR,
        "recall@10 {recall:.3} below floor {RECALL_FLOOR:.3}"
    );
}

#[test]
fn recall_extend_on_beats_extend_off() {
    if cfg!(feature = "simd-simsimd") {
        return;
    }

    let extend_off = mean_recall(NeighborSelectionConfig::default(), 50);
    let extend_on = mean_recall(
        NeighborSelectionConfig {
            extend_candidates: true,
            keep_pruned_connections: true,
        },
        50,
    );

    assert!(
        extend_on + f32::EPSILON >= extend_off,
        "extend_on recall {extend_on:.3} below extend_off {extend_off:.3}"
    );
}

#[test]
fn recall_synthetic_corpus_is_deterministic() {
    let left = synthetic_corpus(256, 8);
    let right = synthetic_corpus(256, 8);

    assert_eq!(left.vectors, right.vectors);
    assert_eq!(left.queries, right.queries);
    assert_eq!(left.layers, right.layers);
    assert_eq!(
        brute_force(&left.vectors, &left.queries[0], 10),
        brute_force(&right.vectors, &right.queries[0], 10)
    );
}

#[test]
fn recall_test_pins_vector_query_and_layer_rngs_independently() {
    assert_eq!(VECTOR_SEED, 0xB65E_0001_u64);
    assert_eq!(QUERY_SEED, 0xB65E_0002_u64);
    assert_eq!(LAYER_SEED, 0xB65E_0003_u64);
    assert_ne!(VECTOR_SEED, QUERY_SEED);
    assert_ne!(VECTOR_SEED, LAYER_SEED);
    assert_ne!(QUERY_SEED, LAYER_SEED);
}

fn mean_recall(neighbor_selection: NeighborSelectionConfig, ef_search: usize) -> f32 {
    let corpus = synthetic_corpus(256, 8);
    let provider = provider_for(&corpus, neighbor_selection, ef_search);
    mean_recall_for_provider(&provider, &corpus, 10, ef_search)
}

fn mean_recall_for_provider(
    provider: &HnswProvider,
    corpus: &SyntheticCorpus,
    k: usize,
    ef_search: usize,
) -> f32 {
    let mut total = 0.0;
    for query in &corpus.queries {
        let exact = brute_force(&corpus.vectors, query, k)
            .into_iter()
            .collect::<HashSet<_>>();
        let approx = provider
            .search(query, k, Some(ef_search), None, None)
            .expect("HNSW search succeeds")
            .into_iter()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>();
        total += approx.intersection(&exact).count() as f32 / k as f32;
    }
    total / corpus.queries.len() as f32
}

fn brute_force(vectors: &[Vec<f32>], query: &[f32], k: usize) -> Vec<NodeId> {
    let mut scored = vectors
        .iter()
        .enumerate()
        .map(|(idx, vector)| {
            (
                NodeId::new((idx + 1) as u64),
                distance(DistanceMetric::L2, query, vector),
            )
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        left.1
            .total_cmp(&right.1)
            .then_with(|| left.0.get().cmp(&right.0.get()))
    });
    scored.into_iter().take(k).map(|(id, _)| id).collect()
}

fn provider_for(
    corpus: &SyntheticCorpus,
    neighbor_selection: NeighborSelectionConfig,
    ef_search: usize,
) -> HnswProvider {
    let config = HnswConfig::with_params(8, 8, 64, ef_search, DistanceMetric::L2)
        .expect("recall config is valid")
        .with_neighbor_selection(neighbor_selection)
        .expect("neighbor selection config is valid");
    let provider = HnswProvider::new(config).expect("provider is valid");
    for (idx, (vector, layer)) in corpus.vectors.iter().zip(&corpus.layers).enumerate() {
        let payload = VectorUpsertPayloadV1 {
            op: VectorOp::Insert,
            node_id: NodeId::new((idx + 1) as u64),
            vector: vector.clone(),
            max_layer: *layer,
        };
        provider
            .on_change(&change(payload))
            .expect("insert applies");
    }
    provider
}

fn change(payload: VectorUpsertPayloadV1) -> Change {
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").expect("provider name interns"),
        payload: Arc::from(
            payload
                .encode()
                .expect("payload encodes")
                .into_boxed_slice(),
        ),
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SyntheticCorpus {
    vectors: Vec<Vec<f32>>,
    queries: Vec<Vec<f32>>,
    layers: Vec<u8>,
}

fn synthetic_corpus(n: usize, dim: usize) -> SyntheticCorpus {
    let mut vector_rng = fastrand::Rng::with_seed(VECTOR_SEED);
    let mut query_rng = fastrand::Rng::with_seed(QUERY_SEED);
    let mut layer_rng = fastrand::Rng::with_seed(LAYER_SEED);
    let params = HnswParams::from_config(
        &HnswConfig::with_params(dim, 8, 64, 50, DistanceMetric::L2)
            .expect("recall params config is valid"),
    );

    SyntheticCorpus {
        vectors: (0..n)
            .map(|_| random_vector(&mut vector_rng, dim))
            .collect(),
        queries: (0..32)
            .map(|_| random_vector(&mut query_rng, dim))
            .collect(),
        layers: (0..n)
            .map(|_| random_layer(&mut layer_rng, params.level_factor))
            .collect(),
    }
}

fn random_vector(rng: &mut fastrand::Rng, dim: usize) -> Vec<f32> {
    (0..dim).map(|_| (rng.f32() * 2.0) - 1.0).collect()
}
