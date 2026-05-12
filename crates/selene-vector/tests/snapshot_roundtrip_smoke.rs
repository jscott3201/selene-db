//! Integration smoke tests for BRIEF-61 snapshot section codecs.

mod common;

use std::sync::Arc;

use common::full_graph_summary;
use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, ProviderError, SubTag};
use selene_vector::{DistanceMetric, HnswConfig, HnswProvider, VectorOp, VectorUpsertPayloadV1};

#[test]
fn empty_graph_roundtrips_through_public_sections() {
    assert_roundtrip(&provider(config(4)), &provider(config(4)));
}

#[test]
fn single_node_roundtrips_through_public_sections() {
    let source = provider(config(4));
    apply_events(&source, &[insert_payload(1, vec![1.0, 0.0, 0.0, 0.0], 0)]);

    assert_roundtrip(&source, &provider(config(4)));
}

#[test]
fn thirty_node_multilayer_graph_roundtrips() {
    let source = provider(config(4));
    let events = deterministic_events(30, 4, 42);
    apply_events(&source, &events);

    assert_roundtrip(&source, &provider(config(4)));
}

#[test]
fn two_hundred_node_multilayer_graph_roundtrips() {
    let source = provider(config(4));
    let events = deterministic_events(200, 4, 4242);
    apply_events(&source, &events);

    assert_roundtrip(&source, &provider(config(4)));
}

#[test]
fn identical_event_sequences_produce_byte_identical_sections() {
    let events = deterministic_events(40, 4, 7);
    let left = provider(config(4));
    let right = provider(config(4));
    apply_events(&left, &events);
    apply_events(&right, &events);

    assert_eq!(snapshot_bytes(&left), snapshot_bytes(&right));
}

#[test]
fn vecs_before_grph_is_rejected() {
    let source = provider(config(4));
    let (_, vecs) = snapshot_bytes(&source);
    let target = provider(config(4));

    let err = target
        .read_section(SubTag(*b"VECS"), &vecs)
        .expect_err("VECS before GRPH is invalid");

    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason } if reason.contains("before GRPH")
    ));
}

#[test]
fn read_rejects_dimension_mismatch() {
    let source = provider(config(4));
    let (grph, _) = snapshot_bytes(&source);
    let target = provider(config(5));

    assert_config_mismatch(target.read_section(SubTag(*b"GRPH"), &grph));
}

#[test]
fn read_rejects_metric_mismatch() {
    let source = provider(config(4));
    let (grph, _) = snapshot_bytes(&source);
    let target = provider(
        HnswConfig::with_params(4, 16, 200, 50, DistanceMetric::L2)
            .expect("mismatch config is valid"),
    );

    assert_config_mismatch(target.read_section(SubTag(*b"GRPH"), &grph));
}

#[test]
fn read_rejects_m_mismatch() {
    let source = provider(config(4));
    let (grph, _) = snapshot_bytes(&source);
    let target = provider(
        HnswConfig::with_params(4, 8, 200, 50, DistanceMetric::Cosine)
            .expect("mismatch config is valid"),
    );

    assert_config_mismatch(target.read_section(SubTag(*b"GRPH"), &grph));
}

#[test]
fn recovery_acceptance_replays_post_snapshot_wal_events() {
    let head = deterministic_events(25, 4, 55);
    let tail = deterministic_events_from(26, 10, 4, 88);
    let source = provider(config(4));
    apply_events(&source, &head);
    let (grph, vecs) = snapshot_bytes(&source);

    let recovered = provider(config(4));
    recovered.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    recovered.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    assert_eq!(
        full_graph_summary(&source.snapshot()),
        full_graph_summary(&recovered.snapshot())
    );

    apply_events(&recovered, &tail);
    let from_scratch = provider(config(4));
    apply_events(&from_scratch, &head);
    apply_events(&from_scratch, &tail);

    assert_eq!(
        full_graph_summary(&from_scratch.snapshot()),
        full_graph_summary(&recovered.snapshot())
    );
}

#[test]
fn write_sections_use_same_captured_arc_after_intervening_change() {
    let source = provider(config(4));
    apply_events(&source, &[insert_payload(1, vec![1.0, 0.0, 0.0, 0.0], 0)]);
    let before = full_graph_summary(&source.snapshot());
    let grph = source.write_section(SubTag(*b"GRPH")).unwrap();

    apply_events(&source, &[insert_payload(2, vec![0.0, 1.0, 0.0, 0.0], 0)]);
    let vecs = source.write_section(SubTag(*b"VECS")).unwrap();

    let recovered = provider(config(4));
    recovered.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    recovered.read_section(SubTag(*b"VECS"), &vecs).unwrap();

    assert_eq!(before, full_graph_summary(&recovered.snapshot()));
    assert_ne!(
        full_graph_summary(&source.snapshot()),
        full_graph_summary(&recovered.snapshot())
    );
}

#[test]
fn staging_clears_after_failed_vecs_read() {
    let source = provider(config(4));
    let (grph, mut vecs) = snapshot_bytes(&source);
    vecs[0] = b'X';
    let target = provider(config(4));

    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target
        .read_section(SubTag(*b"VECS"), &vecs)
        .expect_err("bad VECS fails");
    let (_, good_vecs) = snapshot_bytes(&source);
    target
        .read_section(SubTag(*b"VECS"), &good_vecs)
        .expect_err("staging was cleared after failure");
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &good_vecs).unwrap();
}

fn assert_roundtrip(source: &HnswProvider, target: &HnswProvider) {
    let (grph, vecs) = snapshot_bytes(source);
    assert!(grph.starts_with(b"VGRP"));
    assert!(vecs.starts_with(b"VVEC"));
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    target.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    assert_eq!(
        full_graph_summary(&source.snapshot()),
        full_graph_summary(&target.snapshot())
    );
    assert_eq!((grph, vecs), snapshot_bytes(target));
}

fn snapshot_bytes(provider: &HnswProvider) -> (Vec<u8>, Vec<u8>) {
    let grph = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let vecs = provider.write_section(SubTag(*b"VECS")).unwrap();
    (grph, vecs)
}

fn assert_config_mismatch(result: Result<(), ProviderError>) {
    let err = result.expect_err("config mismatch rejected");
    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("config disagreement")
    ));
}

fn provider(config: HnswConfig) -> HnswProvider {
    HnswProvider::new(config).expect("provider config is valid")
}

fn config(dim: usize) -> HnswConfig {
    HnswConfig::new(dim).expect("test config is valid")
}

fn apply_events(provider: &HnswProvider, events: &[VectorUpsertPayloadV1]) {
    for event in events {
        provider.on_change(&vector_change(event.clone())).unwrap();
    }
}

fn vector_change(payload: VectorUpsertPayloadV1) -> Change {
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").unwrap(),
        payload: Arc::from(payload.encode().unwrap().into_boxed_slice()),
    }
}

fn deterministic_events(count: u64, dim: usize, seed: u64) -> Vec<VectorUpsertPayloadV1> {
    deterministic_events_from(1, count, dim, seed)
}

fn deterministic_events_from(
    start: u64,
    count: u64,
    dim: usize,
    seed: u64,
) -> Vec<VectorUpsertPayloadV1> {
    let mut rng = fastrand::Rng::with_seed(seed);
    (start..start + count)
        .map(|raw| {
            let vector = (0..dim).map(|_| (rng.f32() * 2.0) - 1.0).collect();
            let max_layer = if raw % 11 == 0 {
                2
            } else if raw % 5 == 0 {
                1
            } else {
                0
            };
            insert_payload(raw, vector, max_layer)
        })
        .collect()
}

fn insert_payload(raw: u64, vector: Vec<f32>, max_layer: u8) -> VectorUpsertPayloadV1 {
    VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id: NodeId::new(raw),
        vector,
        max_layer,
    }
}
