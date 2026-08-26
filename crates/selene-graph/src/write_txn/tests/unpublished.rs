use std::sync::{Arc, atomic::AtomicBool};

use parking_lot::Mutex;
use selene_core::{GraphId, LabelSet, NodeId, PropertyMap, Value};

use super::{RecordingProvider, db_string, person_graph_type, prop};
use crate::{IndexProvider, ProviderTag, SharedGraph};

#[test]
fn prepared_snapshot_survives_scratch_rollback_without_publication() {
    let shared = SharedGraph::new(GraphId::new(41));
    let before = shared.read();
    let mut txn = shared.begin_write();
    let node = txn
        .mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();

    let prepared = txn
        .prepare_unpublished(Some(Arc::from([7_u8, 8])), None)
        .unwrap();

    assert_eq!(shared.read().node_count(), 0);
    assert_eq!(shared.read().meta.generation, 0);
    assert!(Arc::ptr_eq(&before, &shared.read()));
    assert!(prepared.snapshot().is_node_alive(node));
    assert_eq!(prepared.snapshot().meta.generation, 1);
    let outcome = prepared.outcome();
    assert_eq!(outcome.generation, 1);
    assert_eq!(outcome.next_node_id, 2);
    assert_eq!(outcome.next_edge_id, 1);
    assert_eq!(outcome.principal.as_deref(), Some(&[7, 8][..]));
    assert_eq!(outcome.changes.len(), 1);
    assert_eq!(outcome.durable_at, None);
}

#[test]
fn preparation_consumes_no_seal_sequence_and_real_commit_does_not_wedge() {
    let shared = SharedGraph::new(GraphId::new(42));
    let mut prepared_txn = shared.begin_write();
    let first = prepared_txn
        .mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    let prepared = prepared_txn.prepare_unpublished(None, None).unwrap();
    assert!(prepared.snapshot().is_node_alive(first));

    let mut real_txn = shared.begin_write();
    let second = real_txn
        .mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    let committer = real_txn.committer.clone();
    let sealed = real_txn.seal(None, None).unwrap();
    assert_eq!(sealed.seal_seq, 0);
    let outcome = committer.submit_commit(sealed).unwrap();

    assert_eq!(outcome.generation, 1);
    assert_eq!(second, NodeId::new(2));
    assert!(!shared.read().is_node_alive(first));
    assert!(shared.read().is_node_alive(second));
}

#[test]
fn cancelled_preparation_rolls_back_without_sequence_or_provider_fanout() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn IndexProvider> = Arc::new(RecordingProvider::new(
        ProviderTag(*b"PREP"),
        Arc::clone(&seen),
    ));
    let shared = SharedGraph::builder(GraphId::new(43))
        .with_provider(provider)
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    let cancelled = AtomicBool::new(true);

    let error = txn
        .prepare_unpublished(None, Some(&cancelled))
        .err()
        .expect("preparation is cancelled");
    assert!(matches!(error, crate::GraphError::Cancelled));
    assert_eq!(shared.read().node_count(), 0);
    assert_eq!(shared.read().meta.generation, 0);
    assert!(seen.lock().is_empty());

    let txn = shared.begin_write();
    let committer = txn.committer.clone();
    let sealed = txn.seal(None, None).unwrap();
    assert_eq!(sealed.seal_seq, 0);
    committer.submit_commit(sealed).unwrap();
}

#[test]
fn validation_failure_during_preparation_publishes_nothing() {
    let shared = SharedGraph::builder(GraphId::new(44))
        .bound_to(person_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let before = shared.read();
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(db_string("Person")),
            prop("name", Value::Int(9)),
        )
        .unwrap();

    let error = txn
        .prepare_unpublished(None, None)
        .err()
        .expect("GG02 validation rejects the staged value");
    assert!(matches!(error, crate::GraphError::TypeViolation(_)));
    assert!(Arc::ptr_eq(&before, &shared.read()));
    assert_eq!(shared.read().meta.generation, 0);
}
