//! Integration smoke tests for BRIEF-63 SQ8 quantization.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use common::full_graph_summary;
use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, ProviderError, SubTag};
use selene_vector::{
    BulkInsertRow, DistanceMetric, HnswConfig, HnswProvider, QuantizationConfig,
    VectorBulkInsertPayloadV1, VectorOp, VectorUpsertPayloadV1,
};

#[test]
fn grph_vecs_alone_recovers_usable_f32_graph() {
    let source = provider(config(4, DistanceMetric::Cosine, true, false));
    apply_events(&source, deterministic_events(20, 4, 11));
    let (grph, vecs, _qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, true, false));

    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();

    assert!(target.quantization_stats().is_none());
    assert_eq!(
        full_graph_summary(&source.snapshot()),
        full_graph_summary(&target.snapshot())
    );
    assert!(
        !target
            .search(&[1.0, 0.0, 0.0, 0.0], 5, Some(20), None)
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

    assert!(target.quantization_stats().is_none());
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

    let stats = target.quantization_stats().expect("QUNT store loaded");
    assert_eq!(stats.code_count, 30);
    assert!(
        !target
            .search(&[0.25, -0.5, 0.75, 0.125], 5, Some(30), None)
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
            .search(&query.vector, 10, Some(256), None)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>();
        let approx = quantized
            .search(&query.vector, 10, Some(256), None)
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
    let rescored = provider(config(8, DistanceMetric::Cosine, true, true));
    apply_events(&source, events.clone());
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    rescored.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    rescored.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    rescored.read_section(SubTag(*b"QUNT"), &qunt).unwrap();

    let exact = source
        .search(&events[10].vector, 10, Some(80), None)
        .unwrap();
    let reranked = rescored
        .search(&events[10].vector, 10, Some(80), None)
        .unwrap();

    assert_eq!(ids(&exact), ids(&reranked));
}

#[test]
fn disabled_read_enabled_snapshot_uses_f32() {
    let source = provider(config(4, DistanceMetric::Cosine, true, false));
    apply_events(&source, deterministic_events(30, 4, 16));
    let (grph, vecs, qunt) = snapshot_bytes(&source);
    let target = provider(config(4, DistanceMetric::Cosine, false, false));
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();

    assert!(target.quantization_stats().is_some());
    assert_eq!(
        source
            .search(&[1.0, 0.0, 0.0, 0.0], 8, Some(30), None)
            .unwrap(),
        target
            .search(&[1.0, 0.0, 0.0, 0.0], 8, Some(30), None)
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

    assert!(target.quantization_stats().is_none());
    assert_eq!(
        source
            .search(&[1.0, 0.0, 0.0, 0.0], 8, Some(30), None)
            .unwrap(),
        target
            .search(&[1.0, 0.0, 0.0, 0.0], 8, Some(30), None)
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

    let stats = recovered.quantization_stats().unwrap();
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

    let stats = recovered.quantization_stats().unwrap();
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

    let results = target.search(&[0.0, 0.0], 2, Some(4), None).unwrap();

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

    assert!(target.quantization_stats().is_none());
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    target.read_section(SubTag(*b"QUNT"), &qunt).unwrap();
    let stats = target.quantization_stats().expect("stats after QUNT");

    assert_eq!(stats.dim, 4);
    assert_eq!(stats.code_count, 8);
    assert_eq!(stats.bytes_codes, 32);
    assert_eq!(stats.bytes_ranges, 32);
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
        .search(&[0.0, 0.0, 0.0, 1.0], 1, Some(20), None)
        .unwrap();

    assert_eq!(results.first().map(|(id, _)| *id), Some(NodeId::new(11)));
    assert_eq!(recovered.quantization_stats().unwrap().code_count, 10);
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
        .with_quantization(QuantizationConfig { enabled, rescore })
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
