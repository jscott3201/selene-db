//! Integration smoke tests for BRIEF-62 bulk insert events.

mod common;

use std::sync::Arc;

use common::full_graph_summary;
use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, ProviderError, SubTag};
use selene_vector::{
    BulkInsertRow, HnswConfig, HnswProvider, VectorBulkInsertPayloadV1, VectorOp,
    VectorUpsertPayloadV1,
};

#[test]
fn bulk_event_matches_equivalent_single_events() {
    let rows = deterministic_rows(1, 30, 4, 42);
    let bulk = provider();
    apply_bulk(&bulk, rows.clone());
    let singles = provider();
    apply_singles(&singles, &rows);

    assert_eq!(snapshot_bytes(&bulk), snapshot_bytes(&singles));
}

#[test]
fn single_row_bulk_matches_single_event() {
    let rows = deterministic_rows(1, 1, 4, 7);
    let bulk = provider();
    apply_bulk(&bulk, rows.clone());
    let singles = provider();
    apply_singles(&singles, &rows);

    assert_eq!(
        full_graph_summary(&bulk.snapshot()),
        full_graph_summary(&singles.snapshot())
    );
}

#[test]
fn bulk_insert_rejects_existing_duplicate_atomically() {
    let provider = provider();
    apply_singles(&provider, &[row(1, vec![1.0, 0.0, 0.0, 0.0], 0)]);
    let before = full_graph_summary(&provider.snapshot());
    let rows = vec![
        row(2, vec![0.0, 1.0, 0.0, 0.0], 0),
        row(3, vec![0.0, 0.0, 1.0, 0.0], 0),
        row(1, vec![0.0, 0.0, 0.0, 1.0], 0),
        row(4, vec![1.0, 1.0, 0.0, 0.0], 0),
        row(5, vec![0.0, 1.0, 1.0, 0.0], 0),
    ];

    let err = provider
        .on_change(&bulk_change(rows))
        .expect_err("existing duplicate rejects whole batch");

    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("apply_bulk_upsert") && reason.contains("DuplicateNodeId")
    ));
    assert_eq!(before, full_graph_summary(&provider.snapshot()));
}

#[test]
fn dimension_error_precedes_existing_duplicate_and_is_atomic() {
    let provider = provider();
    apply_singles(&provider, &[row(1, vec![1.0, 0.0, 0.0, 0.0], 0)]);
    let before = full_graph_summary(&provider.snapshot());

    let err = provider
        .on_change(&bulk_change(vec![row(1, vec![1.0, 0.0, 0.0], 0)]))
        .expect_err("dimension wins before duplicate");

    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("DimensionsLocked") && !reason.contains("DuplicateNodeId")
    ));
    assert_eq!(before, full_graph_summary(&provider.snapshot()));
}

#[test]
fn bulk_empty_payload_encode_rejects_public_api() {
    let err = VectorBulkInsertPayloadV1 { rows: Vec::new() }
        .encode()
        .expect_err("empty batch rejected");

    assert!(matches!(
        err,
        selene_vector::VectorError::InvalidPayload { reason }
            if reason.contains("at least one row")
    ));
}

#[test]
fn mixed_vecu_vecb_recovery_acceptance() {
    let head_single = row(1, vec![1.0, 0.0, 0.0, 0.0], 0);
    let head_bulk = deterministic_rows(2, 12, 4, 99);
    let tail_bulk = deterministic_rows(14, 6, 4, 100);
    let tail_single = row(20, vec![0.0, 0.0, 0.0, 1.0], 1);
    let source = provider();
    apply_singles(&source, std::slice::from_ref(&head_single));
    apply_bulk(&source, head_bulk.clone());
    let (grph, vecs) = snapshot_bytes(&source);

    let recovered = provider();
    recovered.read_section(SubTag(*b"GRPH"), &grph).unwrap();
    recovered.read_section(SubTag(*b"VECS"), &vecs).unwrap();
    apply_bulk(&recovered, tail_bulk.clone());
    apply_singles(&recovered, std::slice::from_ref(&tail_single));

    let from_scratch = provider();
    apply_singles(&from_scratch, &[head_single]);
    apply_bulk(&from_scratch, head_bulk);
    apply_bulk(&from_scratch, tail_bulk);
    apply_singles(&from_scratch, &[tail_single]);

    assert_eq!(
        full_graph_summary(&from_scratch.snapshot()),
        full_graph_summary(&recovered.snapshot())
    );
}

#[test]
fn incomplete_recovery_guard_rejects_vecb_payload() {
    let source = provider();
    apply_singles(&source, &[row(1, vec![1.0, 0.0, 0.0, 0.0], 0)]);
    let (grph, _vecs) = snapshot_bytes(&source);
    let target = provider();
    target.read_section(SubTag(*b"GRPH"), &grph).unwrap();

    let err = target
        .on_change(&bulk_change(vec![row(2, vec![0.0, 1.0, 0.0, 0.0], 0)]))
        .expect_err("VECB must preserve incomplete-recovery guard");

    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("incomplete provider snapshot")
    ));
}

#[test]
fn bulk_insert_two_hundred_rows_succeeds() {
    let provider = provider();
    apply_bulk(&provider, deterministic_rows(1, 200, 4, 777));

    assert_eq!(provider.snapshot().len(), 200);
}

fn provider() -> HnswProvider {
    HnswProvider::new(HnswConfig::new(4).unwrap()).expect("provider config is valid")
}

fn apply_singles(provider: &HnswProvider, rows: &[BulkInsertRow]) {
    for row in rows {
        provider.on_change(&upsert_change(row.clone())).unwrap();
    }
}

fn apply_bulk(provider: &HnswProvider, rows: Vec<BulkInsertRow>) {
    provider.on_change(&bulk_change(rows)).unwrap();
}

fn upsert_change(row: BulkInsertRow) -> Change {
    let payload = VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id: row.node_id,
        vector: row.vector,
        max_layer: row.max_layer,
    };
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

fn snapshot_bytes(provider: &HnswProvider) -> (Vec<u8>, Vec<u8>) {
    let grph = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let vecs = provider.write_section(SubTag(*b"VECS")).unwrap();
    (grph, vecs)
}

fn deterministic_rows(start: u64, count: u64, dim: usize, seed: u64) -> Vec<BulkInsertRow> {
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
            row(raw, vector, max_layer)
        })
        .collect()
}

fn row(raw: u64, vector: Vec<f32>, max_layer: u8) -> BulkInsertRow {
    BulkInsertRow {
        node_id: NodeId::new(raw),
        vector,
        max_layer,
    }
}
