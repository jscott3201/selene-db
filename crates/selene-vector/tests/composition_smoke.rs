//! BRIEF-70 HNSW + quantization composition smoke tests.

use std::collections::HashSet;
use std::sync::Arc;

use roaring::RoaringBitmap;
use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, SubTag};
use selene_vector::distance::distance;
use selene_vector::{
    BulkInsertRow, DistanceMetric, HnswConfig, HnswParams, HnswProvider, PqParams, QuantMethod,
    QuantizationConfig, VectorBulkInsertPayloadV1, VectorOp, VectorUpsertPayloadV1, random_layer,
};

const VECTOR_SEED: u64 = 0xB70E_0001_u64;
const QUERY_SEED: u64 = 0xB70E_0002_u64;
const LAYER_SEED: u64 = 0xB70E_0003_u64;
const REPLAY_PARITY_FLOOR: f32 = 0.90;
const PLAIN_PQ_RECALL_FLOOR: f32 = 0.70;
const OPQ_RESCORE_RECALL_FLOOR: f32 = 0.85;
const POLYSEMOUS_RECALL_FLOOR: f32 = 0.80;
const POLYSEMOUS_DISABLED_DELTA: f32 = 0.05;

#[test]
fn composition_replay_representative_matrix() {
    let cases = [
        Case::new(
            "l2-sq8-bulk",
            DistanceMetric::L2,
            QuantCase::Sq8,
            InsertPattern::BulkBeforeSnapshot,
            false,
        ),
        Case::new(
            "dot-sq8-trickle",
            DistanceMetric::Dot,
            QuantCase::Sq8,
            InsertPattern::SnapshotThenTrickle,
            false,
        ),
        Case::new(
            "cosine-sq8-bulk",
            DistanceMetric::Cosine,
            QuantCase::Sq8,
            InsertPattern::BulkBeforeSnapshot,
            false,
        ),
        Case::new(
            "l2-pq-trickle",
            DistanceMetric::L2,
            QuantCase::PlainPq,
            InsertPattern::SnapshotThenTrickle,
            false,
        ),
        Case::new(
            "dot-sq8-bulk-2",
            DistanceMetric::Dot,
            QuantCase::Sq8,
            InsertPattern::BulkBeforeSnapshot,
            false,
        ),
        Case::new(
            "cosine-sq8-trickle-2",
            DistanceMetric::Cosine,
            QuantCase::Sq8,
            InsertPattern::SnapshotThenTrickle,
            false,
        ),
        Case::new(
            "l2-sq8-bulk-2",
            DistanceMetric::L2,
            QuantCase::Sq8,
            InsertPattern::BulkBeforeSnapshot,
            false,
        ),
        Case::new(
            "dot-sq8-trickle-3",
            DistanceMetric::Dot,
            QuantCase::Sq8,
            InsertPattern::SnapshotThenTrickle,
            false,
        ),
        Case::new(
            "l2-opq-trickle",
            DistanceMetric::L2,
            QuantCase::PqOpq,
            InsertPattern::SnapshotThenTrickle,
            false,
        ),
        Case::new(
            "l2-poly-trickle",
            DistanceMetric::L2,
            QuantCase::PqPolysemous,
            InsertPattern::SnapshotThenTrickle,
            false,
        ),
        Case::new(
            "dot-sq8-bulk-3",
            DistanceMetric::Dot,
            QuantCase::Sq8,
            InsertPattern::BulkBeforeSnapshot,
            false,
        ),
        Case::new(
            "l2-full-filtered",
            DistanceMetric::L2,
            QuantCase::PqOpqPolysemous,
            InsertPattern::SnapshotThenTrickle,
            true,
        ),
    ];

    let corpus = SyntheticCorpus::new(272, 8, 16);
    for (case_index, case) in cases.iter().enumerate() {
        let query = &corpus.queries[case_index % corpus.queries.len()];
        let (direct, recovered, filter) = providers_for_case(&corpus, *case);
        let filter = filter.as_ref();
        let direct_rows = direct
            .search(query, 10, Some(160), filter)
            .unwrap_or_else(|err| panic!("{} direct search failed: {err:?}", case.name));
        let recovered_rows = recovered
            .search(query, 10, Some(160), filter)
            .unwrap_or_else(|err| panic!("{} recovered search failed: {err:?}", case.name));

        assert!(
            !direct_rows.is_empty(),
            "{} direct search returned no rows",
            case.name
        );
        assert!(
            !recovered_rows.is_empty(),
            "{} recovered search returned no rows",
            case.name
        );
        assert!(
            topk_overlap(&direct_rows, &recovered_rows, 10) >= REPLAY_PARITY_FLOOR,
            "{} replay parity below floor: direct={direct_rows:?} recovered={recovered_rows:?}",
            case.name
        );
        if let Some(filter) = filter {
            assert!(
                direct_rows
                    .iter()
                    .all(|(id, _)| filter.contains(id.get() as u32))
            );
            assert!(
                recovered_rows
                    .iter()
                    .all(|(id, _)| filter.contains(id.get() as u32))
            );
        }
    }
}

#[test]
fn composition_oracle_recall_plain_pq_no_rescore_inherits_v44_floor() {
    let recall = mean_exact_recall(QuantCase::PlainPq, false, DistanceMetric::L2);

    assert!(
        recall >= PLAIN_PQ_RECALL_FLOOR,
        "plain PQ recall@10 {recall:.3} below floor {PLAIN_PQ_RECALL_FLOOR:.3}"
    );
}

#[test]
fn composition_oracle_recall_pq_opq_rescore_inherits_v94_floor() {
    let recall = mean_exact_recall(QuantCase::PqOpq, true, DistanceMetric::L2);

    assert!(
        recall >= OPQ_RESCORE_RECALL_FLOOR,
        "OPQ rescore recall@10 {recall:.3} below floor {OPQ_RESCORE_RECALL_FLOOR:.3}"
    );
}

#[test]
fn composition_oracle_recall_polysemous_inherits_v115_delta() {
    let polysemous = mean_exact_recall_on_vector_queries(
        QuantCase::PqPolysemousDefaultThreshold,
        false,
        DistanceMetric::L2,
    );
    let disabled = mean_exact_recall_on_vector_queries(
        QuantCase::PqPolysemousDisabledTwin,
        false,
        DistanceMetric::L2,
    );

    assert!(
        polysemous >= POLYSEMOUS_RECALL_FLOOR,
        "polysemous recall@10 {polysemous:.3} below floor {POLYSEMOUS_RECALL_FLOOR:.3}"
    );
    assert!(
        polysemous + POLYSEMOUS_DISABLED_DELTA >= disabled,
        "polysemous recall@10 {polysemous:.3} is more than {POLYSEMOUS_DISABLED_DELTA:.3} below disabled baseline {disabled:.3}"
    );
}

#[test]
fn composition_replay_with_roaringbitmap_filter_l2_full_stack() {
    let corpus = SyntheticCorpus::new(272, 8, 16);
    let case = Case::new(
        "l2-full-filtered",
        DistanceMetric::L2,
        QuantCase::PqOpqPolysemous,
        InsertPattern::SnapshotThenTrickle,
        true,
    );
    let (direct, recovered, filter) = providers_for_case(&corpus, case);
    let filter = filter.expect("filtered case builds a RoaringBitmap");
    let query = &corpus.queries[0];
    let direct_rows = direct.search(query, 10, Some(160), Some(&filter)).unwrap();
    let recovered_rows = recovered
        .search(query, 10, Some(160), Some(&filter))
        .unwrap();

    assert!(!direct_rows.is_empty());
    assert!(!recovered_rows.is_empty());
    assert!(
        direct_rows
            .iter()
            .all(|(id, _)| filter.contains(id.get() as u32))
    );
    assert!(
        recovered_rows
            .iter()
            .all(|(id, _)| filter.contains(id.get() as u32))
    );
    assert!(topk_overlap(&direct_rows, &recovered_rows, 10) >= REPLAY_PARITY_FLOOR);
}

#[test]
fn composition_build_determinism_qunt_load_before_insert() {
    let corpus = SyntheticCorpus::new(272, 8, 16);
    let left = build_with_quantized_prefix_then_tail(&corpus, QuantCase::PlainPq);
    let right = build_with_quantized_prefix_then_tail(&corpus, QuantCase::PlainPq);
    let left_sections = snapshot_sections(&left);
    let right_sections = snapshot_sections(&right);

    assert_eq!(left_sections.grph, right_sections.grph);
    assert_eq!(left_sections.vecs, right_sections.vecs);
}

#[test]
fn composition_build_determinism_qunt_load_after_insert() {
    let corpus = SyntheticCorpus::new(272, 8, 16);
    let before_tail = build_with_quantized_prefix_then_tail(&corpus, QuantCase::PlainPq);
    let after_tail = build_tail_before_quantization(&corpus, QuantCase::PlainPq);
    let before_sections = snapshot_sections(&before_tail);
    let after_sections = snapshot_sections(&after_tail);

    assert_eq!(before_sections.grph, after_sections.grph);
    assert_eq!(before_sections.vecs, after_sections.vecs);
}

#[test]
fn composition_quantized_prefix_tail_fallback_trickle_insert() {
    let corpus = SyntheticCorpus::new(257, 8, 1);
    let config = config_for(DistanceMetric::L2, QuantCase::PqPolysemousExactOnly, false);
    let provider = HnswProvider::new(config).expect("provider config is valid");
    apply_bulk_rows(&provider, &corpus.vectors[..256], &corpus.layers[..256], 1);
    publish_qunt(&provider);

    let tail_id = NodeId::new(10_001);
    let tail = vec![5.0, -4.0, 3.0, -2.0, 1.0, -1.0, 2.0, -3.0];
    apply_insert(&provider, tail_id, tail.clone(), 0);

    let rows = provider
        .search(&tail, 5, Some(320), None)
        .expect("tail fallback search succeeds");

    assert_eq!(
        rows.first().map(|(id, _)| *id),
        Some(tail_id),
        "tail row beyond quantized prefix must remain admissible under polysemous threshold 0"
    );
}

#[derive(Clone, Copy, Debug)]
struct Case {
    name: &'static str,
    metric: DistanceMetric,
    quant: QuantCase,
    insertion: InsertPattern,
    filtered: bool,
}

impl Case {
    const fn new(
        name: &'static str,
        metric: DistanceMetric,
        quant: QuantCase,
        insertion: InsertPattern,
        filtered: bool,
    ) -> Self {
        Self {
            name,
            metric,
            quant,
            insertion,
            filtered,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum InsertPattern {
    BulkBeforeSnapshot,
    SnapshotThenTrickle,
}

#[derive(Clone, Copy, Debug)]
enum QuantCase {
    Sq8,
    PlainPq,
    PqOpq,
    PqPolysemous,
    PqPolysemousDefaultThreshold,
    PqPolysemousDisabledTwin,
    PqPolysemousExactOnly,
    PqOpqPolysemous,
}

struct Sections {
    grph: Vec<u8>,
    vecs: Vec<u8>,
    qunt: Vec<u8>,
}

#[derive(Clone, Debug)]
struct SyntheticCorpus {
    vectors: Vec<Vec<f32>>,
    queries: Vec<Vec<f32>>,
    layers: Vec<u8>,
    prefix_len: usize,
}

impl SyntheticCorpus {
    fn new(total: usize, dim: usize, tail_len: usize) -> Self {
        let mut vector_rng = fastrand::Rng::with_seed(VECTOR_SEED);
        let mut query_rng = fastrand::Rng::with_seed(QUERY_SEED);
        let mut layer_rng = fastrand::Rng::with_seed(LAYER_SEED);
        let params = HnswParams::from_config(
            &HnswConfig::with_params(dim, 8, 64, 50, DistanceMetric::L2)
                .expect("layer config is valid"),
        );
        Self {
            vectors: (0..total)
                .map(|_| random_vector(&mut vector_rng, dim))
                .collect(),
            queries: (0..32)
                .map(|_| random_vector(&mut query_rng, dim))
                .collect(),
            layers: (0..total)
                .map(|_| random_layer(&mut layer_rng, params.level_factor))
                .collect(),
            prefix_len: total - tail_len,
        }
    }
}

fn providers_for_case(
    corpus: &SyntheticCorpus,
    case: Case,
) -> (HnswProvider, HnswProvider, Option<RoaringBitmap>) {
    let filter = case
        .filtered
        .then(|| even_filter(corpus.vectors.len() as u64));
    match case.insertion {
        InsertPattern::BulkBeforeSnapshot => {
            let config = config_for(case.metric, case.quant, false);
            let direct = HnswProvider::new(config.clone()).expect("provider config is valid");
            apply_bulk_rows(&direct, &corpus.vectors, &corpus.layers, 1);
            let sections = snapshot_sections(&direct);
            let recovered = recover_provider(config, &sections);
            (direct, recovered, filter)
        }
        InsertPattern::SnapshotThenTrickle => {
            let config = config_for(case.metric, case.quant, false);
            let direct = HnswProvider::new(config.clone()).expect("provider config is valid");
            apply_bulk_rows(
                &direct,
                &corpus.vectors[..corpus.prefix_len],
                &corpus.layers[..corpus.prefix_len],
                1,
            );
            let sections = snapshot_sections(&direct);
            apply_tail(&direct, corpus);

            let recovered = recover_provider(config, &sections);
            apply_tail(&recovered, corpus);
            (direct, recovered, filter)
        }
    }
}

fn build_with_quantized_prefix_then_tail(
    corpus: &SyntheticCorpus,
    quant: QuantCase,
) -> HnswProvider {
    let provider =
        HnswProvider::new(config_for(DistanceMetric::L2, quant, false)).expect("config is valid");
    apply_bulk_rows(
        &provider,
        &corpus.vectors[..corpus.prefix_len],
        &corpus.layers[..corpus.prefix_len],
        1,
    );
    publish_qunt(&provider);
    apply_tail(&provider, corpus);
    provider
}

fn build_tail_before_quantization(corpus: &SyntheticCorpus, quant: QuantCase) -> HnswProvider {
    let provider =
        HnswProvider::new(config_for(DistanceMetric::L2, quant, false)).expect("config is valid");
    apply_bulk_rows(&provider, &corpus.vectors, &corpus.layers, 1);
    publish_qunt(&provider);
    provider
}

fn apply_tail(provider: &HnswProvider, corpus: &SyntheticCorpus) {
    let start_id = corpus.prefix_len as u64 + 1;
    apply_bulk_rows(
        provider,
        &corpus.vectors[corpus.prefix_len..],
        &corpus.layers[corpus.prefix_len..],
        start_id,
    );
}

fn mean_exact_recall(quant: QuantCase, rescore: bool, metric: DistanceMetric) -> f32 {
    mean_exact_recall_with_ef(quant, rescore, metric, 50)
}

fn mean_exact_recall_with_ef(
    quant: QuantCase,
    rescore: bool,
    metric: DistanceMetric,
    ef_search: usize,
) -> f32 {
    let corpus = SyntheticCorpus::new(256, 8, 0);
    let provider = HnswProvider::new(config_for(metric, quant, rescore)).expect("config is valid");
    apply_bulk_rows(&provider, &corpus.vectors, &corpus.layers, 1);
    publish_qunt(&provider);

    let mut total = 0.0;
    for query in &corpus.queries {
        let exact = brute_force(&corpus.vectors, query, metric, 10)
            .into_iter()
            .collect::<HashSet<_>>();
        let approx = provider
            .search(query, 10, Some(ef_search), None)
            .expect("composition search succeeds")
            .into_iter()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>();
        total += approx.intersection(&exact).count() as f32 / 10.0;
    }
    total / corpus.queries.len() as f32
}

fn mean_exact_recall_on_vector_queries(
    quant: QuantCase,
    rescore: bool,
    metric: DistanceMetric,
) -> f32 {
    let corpus = SyntheticCorpus::new(256, 8, 0);
    let provider = HnswProvider::new(config_for(metric, quant, rescore)).expect("config is valid");
    apply_bulk_rows(&provider, &corpus.vectors, &corpus.layers, 1);
    publish_qunt(&provider);

    let queries = corpus.vectors.iter().step_by(8).take(32);
    let mut total = 0.0;
    let mut count = 0usize;
    for query in queries {
        let exact = brute_force(&corpus.vectors, query, metric, 10)
            .into_iter()
            .collect::<HashSet<_>>();
        let approx = provider
            .search(query, 10, Some(50), None)
            .expect("composition search succeeds")
            .into_iter()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>();
        total += approx.intersection(&exact).count() as f32 / 10.0;
        count += 1;
    }
    total / count as f32
}

fn brute_force(
    vectors: &[Vec<f32>],
    query: &[f32],
    metric: DistanceMetric,
    k: usize,
) -> Vec<NodeId> {
    let mut scored = vectors
        .iter()
        .enumerate()
        .map(|(idx, vector)| {
            (
                NodeId::new((idx + 1) as u64),
                distance(metric, query, vector),
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

fn config_for(metric: DistanceMetric, quant: QuantCase, rescore: bool) -> HnswConfig {
    HnswConfig::with_params(8, 8, 64, 50, metric)
        .expect("base config is valid")
        .with_quantization(match quant {
            QuantCase::Sq8 => QuantizationConfig {
                enabled: true,
                method: QuantMethod::Sq8,
                rescore,
                pq: None,
            },
            QuantCase::PlainPq => pq_quantization(rescore, 1, false, false, 0.5),
            QuantCase::PqOpq => pq_quantization(rescore, 2, true, false, 0.5),
            QuantCase::PqPolysemous | QuantCase::PqPolysemousDefaultThreshold => {
                pq_quantization(rescore, 2, false, true, 0.5)
            }
            QuantCase::PqPolysemousDisabledTwin => pq_quantization(rescore, 2, false, false, 0.5),
            QuantCase::PqPolysemousExactOnly => pq_quantization(rescore, 2, false, true, 0.0),
            QuantCase::PqOpqPolysemous => pq_quantization(rescore, 2, true, true, 1.0),
        })
        .expect("quantization config is valid")
}

fn pq_quantization(
    rescore: bool,
    m_subspaces: usize,
    use_opq: bool,
    use_polysemous: bool,
    hamming_threshold_ratio: f32,
) -> QuantizationConfig {
    QuantizationConfig {
        enabled: true,
        method: QuantMethod::Pq,
        rescore,
        pq: Some(PqParams {
            m_subspaces,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq,
            use_polysemous,
            hamming_threshold_ratio,
        }),
    }
}

fn snapshot_sections(provider: &HnswProvider) -> Sections {
    let grph = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let vecs = provider.write_section(SubTag(*b"VECS")).unwrap();
    let qunt = provider.write_section(SubTag(*b"QUNT")).unwrap();
    if !qunt.is_empty() {
        provider.read_section(SubTag(*b"QUNT"), &qunt).unwrap();
    }
    Sections { grph, vecs, qunt }
}

fn publish_qunt(provider: &HnswProvider) {
    let _sections = snapshot_sections(provider);
}

fn recover_provider(config: HnswConfig, sections: &Sections) -> HnswProvider {
    let provider = HnswProvider::new(config).expect("provider config is valid");
    provider
        .read_section(SubTag(*b"GRPH"), &sections.grph)
        .unwrap();
    provider
        .read_section(SubTag(*b"VECS"), &sections.vecs)
        .unwrap();
    if !sections.qunt.is_empty() {
        provider
            .read_section(SubTag(*b"QUNT"), &sections.qunt)
            .unwrap();
    }
    provider
}

fn apply_bulk_rows(provider: &HnswProvider, vectors: &[Vec<f32>], layers: &[u8], start_id: u64) {
    let rows = vectors
        .iter()
        .zip(layers)
        .enumerate()
        .map(|(offset, (vector, layer))| BulkInsertRow {
            node_id: NodeId::new(start_id + offset as u64),
            vector: vector.clone(),
            max_layer: *layer,
        })
        .collect();
    provider
        .on_change(&bulk_change(rows))
        .expect("bulk insert applies");
}

fn apply_insert(provider: &HnswProvider, node_id: NodeId, vector: Vec<f32>, max_layer: u8) {
    let payload = VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id,
        vector,
        max_layer,
    };
    provider
        .on_change(&Change::IndexExtensionEvent {
            provider: intern("selene-vector").expect("provider name interns"),
            payload: Arc::from(
                payload
                    .encode()
                    .expect("payload encodes")
                    .into_boxed_slice(),
            ),
        })
        .expect("insert applies");
}

fn bulk_change(rows: Vec<BulkInsertRow>) -> Change {
    let payload = VectorBulkInsertPayloadV1 { rows };
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").expect("provider name interns"),
        payload: Arc::from(
            payload
                .encode()
                .expect("bulk payload encodes")
                .into_boxed_slice(),
        ),
    }
}

fn topk_overlap(left: &[(NodeId, f32)], right: &[(NodeId, f32)], k: usize) -> f32 {
    let left = left
        .iter()
        .take(k)
        .map(|(id, _)| *id)
        .collect::<HashSet<_>>();
    let right = right
        .iter()
        .take(k)
        .map(|(id, _)| *id)
        .collect::<HashSet<_>>();
    left.intersection(&right).count() as f32 / k as f32
}

fn even_filter(max_node_id: u64) -> RoaringBitmap {
    let mut filter = RoaringBitmap::new();
    for raw in 1..=max_node_id {
        if raw.is_multiple_of(2) {
            filter.insert(raw as u32);
        }
    }
    filter
}

fn random_vector(rng: &mut fastrand::Rng, dim: usize) -> Vec<f32> {
    (0..dim).map(|_| (rng.f32() * 2.0) - 1.0).collect()
}
