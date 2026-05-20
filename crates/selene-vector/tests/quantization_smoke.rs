//! Integration smoke tests for BRIEF-63 SQ8 quantization.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use common::full_graph_summary;
use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, ProviderError, SubTag};
use selene_vector::{
    BulkInsertRow, DistanceMetric, HnswConfig, HnswProvider, PqParams, QuantMethod,
    QuantizationConfig, QuantizationStatsKind, VectorBulkInsertPayloadV1, VectorError, VectorOp,
    VectorUpsertPayloadV1,
};

#[test]
fn grph_vecs_alone_recovers_usable_f32_graph() {
    let source = provider(config(4, DistanceMetric::Cosine, true, false));
    apply_events(&source, deterministic_events(20, 4, 11));
    let (grph, vecs, _qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, true, false));

    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();

    assert!(target.quantization_stats().unwrap().is_none());
    assert_eq!(
        full_graph_summary(&source.snapshot()),
        full_graph_summary(&target.snapshot())
    );
    assert!(
        !target
            .search(&[1.0, 0.0, 0.0, 0.0], 5, Some(20), None, None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn qunt_disabled_writes_empty_and_reads_back() {
    let source = provider(config(4, DistanceMetric::Cosine, false, false));
    apply_events(&source, deterministic_events(12, 4, 12));
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, true, false));

    assert!(qunt.is_empty());
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();

    assert!(target.quantization_stats().unwrap().is_none());
    assert_eq!(
        full_graph_summary(&source.snapshot()),
        full_graph_summary(&target.snapshot())
    );
}

#[test]
fn qunt_enabled_full_roundtrip() {
    let source = provider(config(4, DistanceMetric::Cosine, true, false));
    apply_events(&source, deterministic_events(30, 4, 13));
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, true, false));

    assert!(qunt.starts_with(b"VQNT"));
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();

    let stats = target
        .quantization_stats()
        .unwrap()
        .expect("QUNT store loaded");
    assert_eq!(stats.code_count, 30);
    assert!(
        !target
            .search(&[0.25, -0.5, 0.75, 0.125], 5, Some(30), None, None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn asymmetric_recall_at_10_at_least_095_dim16() {
    let source = provider(config(16, DistanceMetric::Cosine, true, false));
    let events = deterministic_events(256, 16, 14);
    apply_events(&source, events.clone());
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let quantized = provider(config(16, DistanceMetric::Cosine, true, false));
    quantized.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    quantized.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    quantized.read_section(SubTag(*b"QUNT"), &qunt).unwrap();

    let mut hits = 0usize;
    let mut total = 0usize;
    for query in events.iter().step_by(12).take(20) {
        let exact = source
            .search(&query.vector, 10, Some(256), None, None)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>();
        let approx = quantized
            .search(&query.vector, 10, Some(256), None, None)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id);
        hits += approx.filter(|id| exact.contains(id)).count();
        total += 10;
    }

    assert!(hits * 100 >= total * 95, "recall was {hits}/{total}");
}

#[test]
fn asymmetric_with_rescore_exact_top_k_constructed_fixture() {
    let events = deterministic_events(80, 8, 15);
    let source = provider(config(8, DistanceMetric::Cosine, true, false));
    let exact_source = provider(config(8, DistanceMetric::Cosine, false, false));
    let rescored = provider(config(8, DistanceMetric::Cosine, true, true));
    apply_events(&source, events.clone());
    apply_events(&exact_source, events.clone());
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    rescored.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    rescored.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    rescored.read_section(SubTag(*b"QUNT"), &qunt).unwrap();

    let exact = exact_source
        .search(&events[10].vector, 10, Some(80), None, None)
        .unwrap();
    let reranked = rescored
        .search(&events[10].vector, 10, Some(80), None, None)
        .unwrap();

    assert_eq!(ids(&exact), ids(&reranked));
}

#[test]
fn disabled_read_enabled_snapshot_uses_f32() {
    let events = deterministic_events(30, 4, 16);
    let source = provider(config(4, DistanceMetric::Cosine, true, false));
    let exact_source = provider(config(4, DistanceMetric::Cosine, false, false));
    apply_events(&source, events.clone());
    apply_events(&exact_source, events);
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, false, false));
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();

    assert!(target.quantization_stats().unwrap().is_some());
    assert_eq!(
        exact_source
            .search(&[1.0, 0.0, 0.0, 0.0], 8, Some(30), None, None)
            .unwrap(),
        target
            .search(&[1.0, 0.0, 0.0, 0.0], 8, Some(30), None, None)
            .unwrap()
    );
}

#[test]
fn enabled_read_disabled_snapshot_uses_f32() {
    let source = provider(config(4, DistanceMetric::Cosine, false, false));
    apply_events(&source, deterministic_events(30, 4, 17));
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, true, false));
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();

    assert!(target.quantization_stats().unwrap().is_none());
    assert_eq!(
        source
            .search(&[1.0, 0.0, 0.0, 0.0], 8, Some(30), None, None)
            .unwrap(),
        target
            .search(&[1.0, 0.0, 0.0, 0.0], 8, Some(30), None, None)
            .unwrap()
    );
}

#[test]
fn qunt_before_complete_graph_rejects_non_empty_body() {
    let source = provider(config(4, DistanceMetric::Cosine, true, false));
    apply_events(&source, deterministic_events(5, 4, 18));
    let (_, _, qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, true, false));

    let err = target
        .read_section(SubTag(*b"QUNT"), &qunt)
        .expect_err("QUNT before graph commit is rejected");

    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason } if reason.contains("node_count")
    ));
}

#[test]
fn qunt_after_grph_without_vecs_preserves_incomplete_recovery_guard() {
    let source = provider(config(4, DistanceMetric::Cosine, true, false));
    apply_events(&source, deterministic_events(5, 4, 23));
    let (grph, _vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, true, false));
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();

    let empty_err = target
        .read_section(SubTag(*b"QUNT"), &Vec::<u8>::new())
        .expect_err("empty QUNT after GRPH-without-VECS rejected");
    assert!(matches!(
        empty_err,
        ProviderError::InvalidPayload { reason } if reason.contains("before VECS")
    ));

    let non_empty_err = target
        .read_section(SubTag(*b"QUNT"), &qunt)
        .expect_err("non-empty QUNT after GRPH-without-VECS rejected");
    assert!(matches!(
        non_empty_err,
        ProviderError::InvalidPayload { reason } if reason.contains("before VECS")
    ));

    let replay_err = target
        .on_change(&upsert_change(insert_payload(99, vec![1.0, 0.0, 0.0, 0.0])))
        .expect_err("BRIEF-61 incomplete-recovery guard still fires after rejected QUNT");
    assert!(matches!(
        replay_err,
        ProviderError::InvalidPayload { reason } if reason.contains("incomplete provider snapshot")
    ));
}

#[test]
fn terminal_state_after_qunt_clears_staging_to_idle() {
    let source = provider(config(4, DistanceMetric::Cosine, true, false));
    apply_events(&source, vec![insert_payload(1, vec![1.0, 0.0, 0.0, 0.0])]);
    let (_grph, _vecs, _qunt) = snapshot_bytes(&source);
    let err = source
        .write_section(SubTag(*b"VECS"))
        .expect_err("VECS after terminal QUNT requires fresh GRPH");
    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason } if reason.contains("before GRPH")
    ));

    apply_events(&source, vec![insert_payload(2, vec![0.0, 1.0, 0.0, 0.0])]);
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, true, false));
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();

    assert_eq!(target.snapshot().len(), 2);
}

#[test]
fn vecu_after_qunt_preserves_quantized_prefix() {
    let recovered = recovered_quantized(10, 4, DistanceMetric::Cosine, 19);
    recovered
        .on_change(&upsert_change(insert_payload(11, vec![0.0, 0.0, 0.0, 1.0])))
        .unwrap();

    let stats = recovered.quantization_stats().unwrap().unwrap();
    assert_eq!(stats.code_count, 10);
    assert_eq!(recovered.snapshot().len(), 11);
}

#[test]
fn vecb_after_qunt_preserves_quantized_prefix() {
    let recovered = recovered_quantized(10, 4, DistanceMetric::Cosine, 20);
    recovered
        .on_change(&bulk_change(vec![
            row(11, vec![0.0, 0.0, 1.0, 0.0]),
            row(12, vec![0.0, 0.0, 0.0, 1.0]),
        ]))
        .unwrap();

    let stats = recovered.quantization_stats().unwrap().unwrap();
    assert_eq!(stats.code_count, 10);
    assert_eq!(recovered.snapshot().len(), 12);
}

#[test]
fn mixed_prefix_l2_score_scale_consistency() {
    let source = provider(config(2, DistanceMetric::L2, true, false));
    apply_events(&source, vec![insert_payload(1, vec![0.0, 0.0])]);
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(2, DistanceMetric::L2, true, false));
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();
    target
        .on_change(&upsert_change(insert_payload(2, vec![3.0, 4.0])))
        .unwrap();

    let results = target.search(&[0.0, 0.0], 2, Some(4), None, None).unwrap();

    assert_eq!(results[0].0, NodeId::new(1));
    assert!((results[0].1 - 0.0).abs() <= 1e-6);
    assert_eq!(results[1].0, NodeId::new(2));
    assert!((results[1].1 - -5.0).abs() <= 1e-6);
}

#[test]
fn quantization_stats_returns_none_before_load() {
    let source = provider(config(4, DistanceMetric::Cosine, true, false));
    apply_events(&source, deterministic_events(8, 4, 21));
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, true, false));

    assert!(target.quantization_stats().unwrap().is_none());
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();
    let stats = target
        .quantization_stats()
        .unwrap()
        .expect("stats after QUNT");

    assert_eq!(stats.dim, 4);
    assert_eq!(stats.code_count, 8);
    assert_eq!(stats.bytes_codes, 32);
    assert_eq!(stats.kind, QuantizationStatsKind::Sq8 { bytes_ranges: 32 });
    assert_eq!(stats.bytes_norms, 32);
    assert!(stats.compression_ratio > 0.0);
}

#[test]
fn bulk_insert_after_snapshot_falls_back_to_f32() {
    let recovered = recovered_quantized(10, 4, DistanceMetric::Cosine, 22);
    recovered
        .on_change(&bulk_change(vec![row(11, vec![0.0, 0.0, 0.0, 1.0])]))
        .unwrap();

    let results = recovered
        .search(&[0.0, 0.0, 0.0, 1.0], 1, Some(20), None, None)
        .unwrap();

    assert_eq!(results.first().map(|(id, _)| *id), Some(NodeId::new(11)));
    assert_eq!(
        recovered.quantization_stats().unwrap().unwrap().code_count,
        10
    );
}

#[test]
fn pq_write_qunt_trains_and_publishes_store() {
    let source = provider(config_pq(8, DistanceMetric::L2, false, 256));
    apply_events(&source, deterministic_events(256, 8, 66));

    let (_grph, _vecs, qunt) = snapshot_bytes(&source);

    assert!(qunt.starts_with(b"VQNT"));
    let stats = source
        .quantization_stats()
        .unwrap()
        .expect("PQ store published after QUNT write");
    assert_eq!(stats.method, QuantMethod::Pq);
    assert_eq!(stats.code_count, 256);
    assert!(matches!(
        stats.kind,
        QuantizationStatsKind::Pq {
            bytes_codebook,
            bytes_rotation,
            polysemous: false,
        } if bytes_codebook > 0 && bytes_rotation == 0
    ));
}

#[test]
fn pq_opq_stats_report_plain_winner_without_rotation_bytes() {
    let config = HnswConfig::with_params(8, 16, 200, 50, DistanceMetric::L2)
        .unwrap()
        .with_quantization(QuantizationConfig {
            enabled: true,
            method: QuantMethod::Pq,
            pq: Some(PqParams {
                m_subspaces: 2,
                k_centroids: 256,
                train_min_vectors: 256,
                use_opq: true,
                use_polysemous: false,
                hamming_threshold_ratio: 0.5,
            }),
            ..Default::default()
        })
        .unwrap();
    let source = provider(config);
    apply_events(&source, deterministic_events(256, 8, 166));

    let (_grph, _vecs, qunt) = snapshot_bytes(&source);

    assert!(qunt.starts_with(b"VQNT"));
    let stats = source
        .quantization_stats()
        .unwrap()
        .expect("OPQ store published after QUNT write");
    assert!(matches!(
        stats.kind,
        QuantizationStatsKind::Pq {
            bytes_codebook,
            bytes_rotation,
            polysemous: false,
        } if bytes_codebook > 0 && bytes_rotation == 0
    ));
}

#[test]
fn pq_write_qunt_below_threshold_emits_empty_and_stats_deferred() {
    let source = provider(config_pq(8, DistanceMetric::L2, false, 256));
    apply_events(&source, deterministic_events(100, 8, 67));

    let (_grph, _vecs, qunt) = snapshot_bytes(&source);
    let err = source
        .quantization_stats()
        .expect_err("PQ stats deferred until enough vectors exist");

    assert!(qunt.is_empty());
    assert!(matches!(
        err,
        VectorError::PqTrainingDeferred {
            observed_vectors: 100,
            required: 256
        }
    ));
}

#[test]
fn pq_stats_returns_deferred_below_threshold_without_store() {
    let source = provider(config_pq(8, DistanceMetric::L2, false, 256));
    apply_events(&source, deterministic_events(100, 8, 69));

    let err = source
        .quantization_stats()
        .expect_err("PQ stats deferred below training threshold");

    assert!(matches!(
        err,
        VectorError::PqTrainingDeferred {
            observed_vectors: 100,
            required: 256
        }
    ));
}

#[test]
fn pq_stats_returns_ok_none_above_threshold_without_store() {
    let source = provider(config_pq(8, DistanceMetric::L2, false, 256));
    apply_events(&source, deterministic_events(300, 8, 70));

    assert!(
        source
            .quantization_stats()
            .expect("above-threshold PQ without QUNT is transient no-store")
            .is_none()
    );
}

#[test]
fn pq_search_falls_back_to_f32_when_training_deferred() {
    let pq = provider(config_pq(8, DistanceMetric::L2, false, 256));
    let f32 = provider(config(8, DistanceMetric::L2, false, false));
    let events = deterministic_events(100, 8, 68);
    apply_events(&pq, events.clone());
    apply_events(&f32, events);
    let query = [0.2, -0.4, 0.1, 0.8, -0.2, 0.3, -0.7, 0.5];

    assert!(matches!(
        pq.quantization_stats(),
        Err(VectorError::PqTrainingDeferred { .. })
    ));
    assert_eq!(
        pq.search(&query, 8, Some(80), None, None).unwrap(),
        f32.search(&query, 8, Some(80), None, None).unwrap()
    );
}

#[test]
fn pq_dim_not_divisible_by_m_rejected_at_validate() {
    let err = HnswConfig::with_params(10, 16, 200, 50, DistanceMetric::L2)
        .unwrap()
        .with_quantization(QuantizationConfig {
            enabled: true,
            method: QuantMethod::Pq,
            pq: Some(PqParams {
                m_subspaces: 3,
                k_centroids: 256,
                train_min_vectors: 256,
                use_opq: false,
                use_polysemous: false,
                hamming_threshold_ratio: 0.5,
            }),
            ..Default::default()
        })
        .expect_err("PQ dim divisibility rejected");

    assert!(matches!(
        err,
        VectorError::PqDimensionNotDivisible {
            dim: 10,
            m_subspaces: 3
        }
    ));
}

fn recovered_quantized(count: u64, dim: usize, metric: DistanceMetric, seed: u64) -> HnswProvider {
    let source = provider(config(dim, metric, true, false));
    apply_events(&source, deterministic_events(count, dim, seed));
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(dim, metric, true, false));
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();
    target
}

fn config(dim: usize, metric: DistanceMetric, enabled: bool, rescore: bool) -> HnswConfig {
    HnswConfig::with_params(dim, 16, 200, dim.max(50), metric)
        .unwrap()
        .with_quantization(QuantizationConfig {
            enabled,
            rescore,
            ..Default::default()
        })
        .unwrap()
}

fn config_pq(
    dim: usize,
    metric: DistanceMetric,
    rescore: bool,
    train_min_vectors: usize,
) -> HnswConfig {
    HnswConfig::with_params(dim, 16, 200, dim.max(50), metric)
        .unwrap()
        .with_quantization(QuantizationConfig {
            enabled: true,
            method: QuantMethod::Pq,
            rescore,
            pq: Some(PqParams {
                m_subspaces: 1,
                k_centroids: 256,
                train_min_vectors,
                use_opq: false,
                use_polysemous: false,
                hamming_threshold_ratio: 0.5,
            }),
        })
        .unwrap()
}

fn provider(config: HnswConfig) -> HnswProvider {
    HnswProvider::new(config).expect("provider config is valid")
}

fn apply_events(provider: &HnswProvider, events: Vec<VectorUpsertPayloadV1>) {
    for event in events {
        provider.on_change(&upsert_change(event)).unwrap();
    }
}

fn upsert_change(payload: VectorUpsertPayloadV1) -> Change {
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").unwrap(),
        payload: Arc::from(payload.encode().unwrap().into_boxed_slice()),
    }
}

fn bulk_change(rows: Vec<BulkInsertRow>) -> Change {
    let payload = VectorBulkInsertPayloadV1 { rows };
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").unwrap(),
        payload: Arc::from(payload.encode().unwrap().into_boxed_slice()),
    }
}

fn snapshot_bytes(provider: &HnswProvider) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let grph = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let vecs = provider.write_section(SubTag(*b"VECS")).unwrap();
    let qunt = provider.write_section(SubTag(*b"QUNT")).unwrap();
    (grph, vecs, qunt)
}

fn deterministic_events(count: u64, dim: usize, seed: u64) -> Vec<VectorUpsertPayloadV1> {
    let mut rng = fastrand::Rng::with_seed(seed);
    (1..=count)
        .map(|raw| {
            let vector = (0..dim).map(|_| (rng.f32() * 2.0) - 1.0).collect();
            insert_payload(raw, vector)
        })
        .collect()
}

fn insert_payload(raw: u64, vector: Vec<f32>) -> VectorUpsertPayloadV1 {
    VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id: NodeId::new(raw),
        vector,
        max_layer: 0,
    }
}

fn row(raw: u64, vector: Vec<f32>) -> BulkInsertRow {
    BulkInsertRow {
        node_id: NodeId::new(raw),
        vector,
        max_layer: 0,
    }
}

fn ids(results: &[(NodeId, f32)]) -> Vec<NodeId> {
    results.iter().map(|(id, _)| *id).collect()
}
