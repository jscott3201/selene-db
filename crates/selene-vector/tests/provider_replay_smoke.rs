//! Integration smoke tests for BRIEF-59 provider replay.

mod common;

use std::sync::Arc;

use common::graph_summary;
use selene_core::{Change, GraphId, LabelSet, NodeId, PropertyMap, intern};
use selene_graph::{IndexProvider, ProviderError, SharedGraph};
use selene_vector::{HnswConfig, HnswProvider, VectorOp, VectorUpsertPayloadV1};

#[test]
fn on_change_ignores_non_vector_provider_event() {
    let provider = provider();
    let prev = provider.snapshot();
    let change = Change::IndexExtensionEvent {
        provider: intern("selene-other").unwrap(),
        payload: Arc::from(b"not-vector".as_slice()),
    };

    provider
        .on_change(&change)
        .expect("other provider is ignored");

    assert!(Arc::ptr_eq(&prev, &provider.snapshot()));
}

#[test]
fn on_change_ignores_non_extension_change() {
    let provider = provider();
    let prev = provider.snapshot();
    let change = Change::NodeCreated {
        id: NodeId::new(1),
        labels: LabelSet::new(),
        properties: PropertyMap::new(),
    };

    provider
        .on_change(&change)
        .expect("non-extension is ignored");

    assert!(Arc::ptr_eq(&prev, &provider.snapshot()));
}

#[test]
fn on_change_applies_insert_and_publishes_new_arc() {
    let provider = provider();
    let prev = provider.snapshot();
    let change = vector_change(insert_payload(NodeId::new(1), vec![1.0, 0.0, 0.0, 0.0], 0));

    provider.on_change(&change).expect("insert is applied");
    let post = provider.snapshot();

    assert!(!Arc::ptr_eq(&prev, &post));
    assert_eq!(post.len(), 1);
}

#[test]
fn on_change_propagates_decode_failure_as_invalid_payload() {
    let provider = provider();
    let change = Change::IndexExtensionEvent {
        provider: intern("selene-vector").unwrap(),
        payload: Arc::from(b"XXXX".as_slice()),
    };

    let err = provider
        .on_change(&change)
        .expect_err("decode failure is returned");

    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("InvalidPayload") && reason.contains("payload decode")
    ));
}

#[test]
fn on_change_propagates_dimension_lockout() {
    let provider = provider();
    let change = vector_change(insert_payload(NodeId::new(1), vec![1.0, 2.0, 3.0], 0));

    let err = provider
        .on_change(&change)
        .expect_err("dimension lockout is returned");

    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("DimensionsLocked") && reason.contains("apply_upsert")
    ));
}

#[test]
fn on_change_propagates_update_not_supported() {
    let provider = provider();
    let change = vector_change(VectorUpsertPayloadV1 {
        op: VectorOp::Update,
        node_id: NodeId::new(1),
        vector: vec![1.0, 0.0, 0.0, 0.0],
        max_layer: 0,
    });

    let err = provider.on_change(&change).expect_err("update is deferred");

    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("OperationNotSupportedYet") && reason.contains("future")
    ));
}

#[test]
fn on_change_propagates_delete_not_supported() {
    let provider = provider();
    let change = vector_change(VectorUpsertPayloadV1 {
        op: VectorOp::Delete,
        node_id: NodeId::new(1),
        vector: Vec::new(),
        max_layer: 0,
    });

    let err = provider.on_change(&change).expect_err("delete is deferred");

    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("OperationNotSupportedYet") && reason.contains("future")
    ));
}

#[test]
fn replay_sequence_produces_deterministic_graph() {
    let events = deterministic_events();
    let left = provider();
    let right = provider();

    for event in &events {
        left.on_change(&vector_change(event.clone())).unwrap();
        right.on_change(&vector_change(event.clone())).unwrap();
    }

    assert_eq!(
        graph_summary(&left.snapshot()),
        graph_summary(&right.snapshot())
    );
}

#[test]
fn commit_through_mutator_extension_event_publishes_new_snapshot() {
    let provider = Arc::new(provider());
    let provider_for_graph: Arc<dyn IndexProvider> = provider.clone();
    let graph = SharedGraph::builder(GraphId::new(1))
        .with_provider(provider_for_graph)
        .build()
        .expect("graph builds");
    let prev = provider.snapshot();
    let mut txn = graph.begin_write();
    let payload = insert_payload(NodeId::new(1), vec![1.0, 0.0, 0.0, 0.0], 0)
        .encode()
        .unwrap();

    txn.mutator().extension_event(
        intern("selene-vector").unwrap(),
        Arc::from(payload.into_boxed_slice()),
    );
    txn.commit().expect("commit succeeds");
    let post = provider.snapshot();

    assert!(!Arc::ptr_eq(&prev, &post));
    assert_eq!(post.len(), 1);
}

#[test]
fn provider_search_after_inserts_returns_recent_node_first() {
    let provider = provider();
    let events = [
        (NodeId::new(1), vec![1.0, 0.0, 0.0, 0.0]),
        (NodeId::new(2), vec![0.0, 1.0, 0.0, 0.0]),
        (NodeId::new(3), vec![0.0, 0.0, 1.0, 0.0]),
        (NodeId::new(4), vec![1.0, 1.0, 0.0, 0.0]),
        (NodeId::new(5), vec![0.0, 0.0, 0.0, 1.0]),
    ];
    for (node_id, vector) in events {
        provider
            .on_change(&vector_change(insert_payload(node_id, vector, 0)))
            .unwrap();
    }

    let results = provider
        .search(&[0.0, 0.0, 0.0, 1.0], 1, Some(8), None)
        .unwrap();

    assert_eq!(results.first().map(|(id, _)| *id), Some(NodeId::new(5)));
}

#[test]
fn provider_search_uses_config_ef_when_override_none() {
    let provider = provider();
    provider
        .on_change(&vector_change(insert_payload(
            NodeId::new(1),
            vec![1.0, 0.0, 0.0, 0.0],
            0,
        )))
        .unwrap();

    let results = provider
        .search(&[1.0, 0.0, 0.0, 0.0], 3, None, None)
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, NodeId::new(1));
}

fn provider() -> HnswProvider {
    HnswProvider::new(HnswConfig::new(4).unwrap()).expect("provider config is valid")
}

fn vector_change(payload: VectorUpsertPayloadV1) -> Change {
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").unwrap(),
        payload: Arc::from(payload.encode().unwrap().into_boxed_slice()),
    }
}

fn insert_payload(node_id: NodeId, vector: Vec<f32>, max_layer: u8) -> VectorUpsertPayloadV1 {
    VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id,
        vector,
        max_layer,
    }
}

fn deterministic_events() -> Vec<VectorUpsertPayloadV1> {
    let mut rng = fastrand::Rng::with_seed(42);
    (1..=10)
        .map(|raw| {
            let vector = (0..4).map(|_| (rng.f32() * 2.0) - 1.0).collect();
            let max_layer = if raw % 5 == 0 { 1 } else { 0 };
            insert_payload(NodeId::new(raw), vector, max_layer)
        })
        .collect()
}
