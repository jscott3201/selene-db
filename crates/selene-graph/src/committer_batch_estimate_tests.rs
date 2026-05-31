//! v1.2 multi-writer BRIEF 2 — F4 byte-cap + `encoded_estimate` tests (GRAPH-11).
//!
//! Child of `committer_batch_tests.rs`; reuses its helpers + synthetic durable
//! providers via `use super::*`. Split into its own file to keep each test file
//! under the 700-LOC cap. These pin the parts of the F4 group-commit cap that
//! `committer_batch_tests`/`_wal_tests` left uncovered:
//!
//! - [`super::super::encoded_estimate`] — the exact `64 + n*256` formula the
//!   aggregate-byte cap is computed from (the count cap is a separate axis, T13b).
//! - the **aggregate**-byte cap binding mid-run (a batch of >1 but bounded
//!   *below* the count cap purely by accumulated bytes — distinct from T13 where
//!   every commit is individually over the tiny cap).
//! - the `>= 1` progress rule: a single commit whose estimate exceeds `max_bytes`
//!   is taken ALONE and committed, never rejected.

use std::sync::Arc;
use std::thread;

use selene_core::{LabelSet, PropertyMap};

use super::*;
use crate::committer_batch::{CommitBatching, encoded_estimate};

// ───────────────────────── encoded_estimate formula ─────────────────────────

#[test]
fn encoded_estimate_is_header_plus_per_change_allowance() {
    // The F4 aggregate-byte cap is computed from this estimate, so its formula is
    // load-bearing for WHEN a barrier fires. Pin it directly: 64-byte per-commit
    // header + 256 bytes per Change. A drift in either constant (or a switch to
    // the real encoded length) changes the batch-coalescing point and must be a
    // deliberate, test-updating change — not a silent one.
    const HEADER: u64 = 64;
    const PER_CHANGE: u64 = 256;
    let shared = graph_with_durable(70_080, CountingDurable::new(b"EST0"), CommitBatching::Off);

    // 0 changes (a no-op-shaped seal still carries the header allowance), 1
    // change, and 3 changes — assert the linear formula at each point.
    for change_count in [0_usize, 1, 3] {
        let mut txn = shared.begin_write();
        {
            let mut m = txn.mutator();
            for _ in 0..change_count {
                m.create_node(LabelSet::single(istr("Est")), PropertyMap::new())
                    .unwrap();
            }
        }
        let sealed = txn.seal(None, None).expect("seals");
        assert_eq!(
            sealed.changes.len(),
            change_count,
            "seal carried the expected change count",
        );
        assert_eq!(
            encoded_estimate(&sealed),
            HEADER + PER_CHANGE * change_count as u64,
            "encoded_estimate must equal 64 + 256*n for n={change_count} changes",
        );
        // Don't publish — drop the sealed commit (the committer never sees it).
        drop(sealed);
    }
}

#[test]
fn encoded_estimate_saturates_on_pathological_change_count() {
    // Defense-in-depth: the estimate uses saturating arithmetic so a degenerate
    // change count can never wrap to a small value and defeat the byte cap. A
    // realistic commit can't reach u64::MAX changes, so assert the helper's
    // saturation contract via the formula at a large-but-buildable commit and
    // rely on the saturating_mul/_add in the impl for the extreme.
    let shared = graph_with_durable(70_081, CountingDurable::new(b"EST1"), CommitBatching::Off);
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        for _ in 0..100 {
            m.create_node(LabelSet::single(istr("Sat")), PropertyMap::new())
                .unwrap();
        }
    }
    let sealed = txn.seal(None, None).expect("seals");
    assert_eq!(encoded_estimate(&sealed), 64 + 256 * 100);
}

// ───────────────────────── aggregate byte cap ─────────────────────────

#[test]
fn aggregate_byte_cap_bounds_batch_below_count_cap() {
    // The aggregate (not per-commit) byte cap must end a run even when the COUNT
    // cap is nowhere near. With max_commits = 64 but max_bytes = 700, each
    // 1-change commit estimates 64 + 256 = 320, so two fit (640 <= 700) but a
    // third would push the run to 960 > 700 → the run is flushed at 2. This is
    // distinct from T13 (where the cap is so tiny EVERY commit is over it alone):
    // here the cap binds at an aggregate of TWO sub-count-cap commits.
    const TOTAL: usize = 6;
    let durable = CountingDurable::new(b"BCAP");
    let shared = Arc::new(graph_with_durable(70_082, durable.clone(), on(64, 700)));

    // Buffer seqs 1..TOTAL-1 behind the seq-0 gap so the whole run is present in
    // the reorder buffer when drain runs (mirrors T13b), then release seq 0.
    let mut sealeds = Vec::new();
    for _ in 0..TOTAL {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("B")), PropertyMap::new())
            .unwrap();
        sealeds.push(txn.seal(None, None).expect("seals"));
    }
    let sealed_0 = sealeds.remove(0);
    let mut handles = Vec::new();
    while let Some(sealed) = sealeds.pop() {
        let shared = Arc::clone(&shared);
        handles.push(thread::spawn(move || {
            shared
                .submit_sealed_for_test(sealed)
                .expect("buffered commit")
        }));
        for _ in 0..200 {
            thread::yield_now();
        }
    }
    shared.submit_sealed_for_test(sealed_0).expect("seq 0");
    for handle in handles {
        handle.join().expect("waiter ok");
    }

    assert_eq!(shared.read().node_count(), TOTAL, "no loss");
    assert_eq!(durable.write_count(), TOTAL, "every commit appended once");
    // The aggregate-byte cap bounds the run at exactly 2 (640 <= 700 < 960),
    // strictly below the count cap of 64.
    assert_eq!(
        durable.max_batch_size(),
        2,
        "aggregate byte cap (700) bounds the run to 2 of 6 buffered commits (observed {})",
        durable.max_batch_size(),
    );
}

// ───────────────────────── >= 1 progress rule ─────────────────────────

#[test]
fn over_cap_single_commit_is_taken_alone_and_committed() {
    // A single commit whose estimate exceeds max_bytes must still commit (taken
    // ALONE), never rejected — the byte cap bounds accumulation, not commit size.
    // 50 changes estimate 64 + 50*256 = 12_864, far over the tiny 80-byte cap and
    // also over the count cap, so it can only succeed via the >= 1 progress rule.
    let durable = CountingDurable::new(b"OVR1");
    let shared = graph_with_durable(70_083, durable.clone(), on(8, 80));

    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        for _ in 0..50 {
            m.create_node(LabelSet::single(istr("Fat")), PropertyMap::new())
                .unwrap();
        }
    }
    let outcome = txn
        .commit()
        .expect("over-cap commit is taken alone, never rejected");
    assert_eq!(
        outcome.durable_at,
        Some(1),
        "the lone over-cap commit is durable"
    );
    assert_eq!(shared.read().node_count(), 50);
    // It rode the batched path as a single-member run: exactly one write + one
    // flush, and the largest observed batch is 1.
    assert_eq!(durable.write_count(), 1);
    assert_eq!(durable.flush_count(), 1);
    assert_eq!(durable.max_batch_size(), 1, "over-cap commit taken alone");
}
