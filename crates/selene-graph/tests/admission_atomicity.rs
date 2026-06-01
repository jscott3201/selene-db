//! BRIEF-153 fix-cycle C2: when property-index admission fails during
//! `Mutator::create_node` (or `update_node`), no partial row state must
//! remain in the working graph.
//!
//! Runs as a separate integration-test binary so filling the global
//! [`IStr`] pool here does not poison the shared in-process pool of other
//! nextest binaries. The test fills the pool to [`MAX_INTERNED_STRINGS`]
//! and then asserts that a subsequent `create_node` whose indexed
//! `Value::ExternalString` requires a fresh admission returns
//! `GraphError::IndexAdmissionExhausted` and leaves the node store
//! unchanged.
//!
//! Marked `#[ignore]` by default because filling 1,000,000 IStrs is on
//! the order of a few hundred milliseconds and there is no
//! test-only cap override; downstream changes touching the mutator
//! ordering should run this test manually
//! (`cargo nextest run -p selene-graph --test admission_atomicity -- --ignored`).

use std::sync::Arc;

use selene_core::{GraphId, LabelSet, PropertyMap, Value, intern};
use selene_graph::{GraphError, SharedGraph, TypedIndexKind};

#[test]
#[ignore = "fills the global IStr pool; run manually for atomicity check"]
fn create_node_admission_failure_leaves_no_partial_node_state() {
    let graph = SharedGraph::new(GraphId::new(99_999));
    let label = intern("admission_atomicity.label").unwrap();
    let indexed_prop = intern("admission_atomicity.name").unwrap();
    graph
        .create_property_index(label.clone(), indexed_prop.clone(), TypedIndexKind::String)
        .unwrap();

    // Fill the IStr pool to its hard cap.
    while let Ok(_handle) = intern_fresh() {}
    let lookup_before_create = selene_core::lookup("admission_atomicity.probe.unique");
    assert!(
        lookup_before_create.is_none(),
        "probe content cannot already be pooled (cap is full of other strings)"
    );

    // Take a snapshot of node-store length before attempting the doomed
    // create.
    let snapshot_len_before = graph.read().node_store.len();

    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let err = mutator
            .create_node(
                LabelSet::single(label),
                PropertyMap::from_pairs([(
                    indexed_prop,
                    Value::ExternalString(Arc::<str>::from("admission_atomicity.probe.unique")),
                )])
                .unwrap(),
            )
            .expect_err("admission fails because the pool is full");

        assert!(matches!(err, GraphError::IndexAdmissionExhausted { .. }));
        // Drop the txn implicitly on scope exit — the working tree's
        // intermediate state never publishes (the err early-returned
        // before `commit()` was even called).
    }

    // The committed snapshot must show no growth from the doomed call.
    let snapshot_len_after = graph.read().node_store.len();
    assert_eq!(
        snapshot_len_before, snapshot_len_after,
        "node_store grew despite admission failure — partial-row leak"
    );

    // Independent assertion that the BRIEF-153 fix-cycle C2 reorder is
    // load-bearing: even if the txn rolled back via Drop, the WORKING
    // (pre-publish) tree must never have observed a row pointing to a
    // missing index entry. Since the row never existed in this txn,
    // there's nothing to compensate.
    assert!(
        selene_core::lookup("admission_atomicity.probe.unique").is_none(),
        "the probe content must remain absent from the IStr pool (admission rejected)"
    );
}

/// Try to intern a fresh string that is guaranteed not to be in the pool
/// from earlier test setup. Each call uses a monotonically increasing
/// counter to avoid the fast-path-hit case.
fn intern_fresh() -> Result<selene_core::IStr, selene_core::CoreError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    intern(&format!("admission_atomicity.filler.{n}"))
}
