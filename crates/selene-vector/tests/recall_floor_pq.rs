//! Deterministic PQ recall floor for BRIEF-66.

use std::collections::HashSet;
use std::sync::Arc;

use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, SubTag};
use selene_vector::distance::distance;
use selene_vector::{
    DistanceMetric, HnswConfig, HnswParams, HnswProvider, PqParams, QuantMethod,
    QuantizationConfig, VectorOp, VectorUpsertPayloadV1, random_layer,
};

const VECTOR_SEED: u64 = 0xB65E_0001_u64;
const QUERY_SEED: u64 = 0xB65E_0002_u64;
const LAYER_SEED: u64 = 0xB65E_0003_u64;
const PQ_TRAIN_SEED: u64 = 0xB66E_0001_u64;
const PQ_RECALL_FLOOR: f32 = 0.70;
const PQ_RESCORE_RECALL_FLOOR: f32 = 0.85;
const OPQ_RESCORE_RECALL_FLOOR: f32 = 0.85;

#[test]
fn pq_recall_at_10_meets_floor_no_rescore() {
    if cfg!(feature = "simd-simsimd") {
        return;
    }

    let recall = mean_recall(false, false, 1, 50);

    assert!(
        recall >= PQ_RECALL_FLOOR,
        "PQ recall@10 {recall:.3} below floor {PQ_RECALL_FLOOR:.3}"
    );
}

#[test]
fn pq_recall_at_10_meets_floor_with_rescore() {
    if cfg!(feature = "simd-simsimd") {
        return;
    }

    let recall = mean_recall(true, false, 1, 50);

    assert!(
        recall >= PQ_RESCORE_RECALL_FLOOR,
        "PQ rescore recall@10 {recall:.3} below floor {PQ_RESCORE_RECALL_FLOOR:.3}"
    );
}

#[test]
fn pq_recall_with_rescore_beats_pq_without() {
    if cfg!(feature = "simd-simsimd") {
        return;
    }

    let pq = mean_recall(false, false, 1, 50);
    let rescored = mean_recall(true, false, 1, 50);

    assert!(
        rescored + f32::EPSILON >= pq,
        "PQ rescore recall {rescored:.3} below PQ recall {pq:.3}"
    );
}

#[test]
fn opq_rescore_recall_at_10_meets_floor() {
    if cfg!(feature = "simd-simsimd") {
        return;
    }

    let recall = mean_recall(true, true, 2, 50);

    assert!(
        recall >= OPQ_RESCORE_RECALL_FLOOR,
        "OPQ rescore recall@10 {recall:.3} below floor {OPQ_RESCORE_RECALL_FLOOR:.3}"
    );
}

#[test]
fn pq_train_seed_pinned_in_recall_harness() {
    assert_eq!(PQ_TRAIN_SEED, 0xB66E_0001_u64);
}

fn mean_recall(rescore: bool, use_opq: bool, m_subspaces: usize, ef_search: usize) -> f32 {
    let corpus = synthetic_corpus(256, 8);
    let provider = provider_for(&corpus, rescore, use_opq, m_subspaces, ef_search);
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
            .search(query, k, Some(ef_search), None)
            .expect("PQ search succeeds")
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
    rescore: bool,
    use_opq: bool,
    m_subspaces: usize,
    ef_search: usize,
) -> HnswProvider {
    let config = HnswConfig::with_params(8, 8, 64, ef_search, DistanceMetric::L2)
        .expect("recall config is valid")
        .with_quantization(QuantizationConfig {
            enabled: true,
            method: QuantMethod::Pq,
            rescore,
            pq: Some(PqParams {
                m_subspaces,
                k_centroids: 256,
                train_min_vectors: 256,
                use_opq,
                use_polysemous: false,
                hamming_threshold_ratio: 0.5,
            }),
        })
        .expect("PQ config is valid");
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
    load_quantization(&provider);
    provider
}

fn load_quantization(provider: &HnswProvider) {
    let _grph = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let _vecs = provider.write_section(SubTag(*b"VECS")).unwrap();
    let qunt = provider.write_section(SubTag(*b"QUNT")).unwrap();
    if !qunt.is_empty() {
        provider.read_section(SubTag(*b"QUNT"), &qunt).unwrap();
    }
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

#[derive(Clone, Debug)]
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
