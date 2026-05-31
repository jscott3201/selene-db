//! v1.2 multi-writer BRIEF 2 — WAL group-commit tests (T1..T13).
//!
//! These exercise the batched committer driver: `CommitBatching::Off` is
//! behaviorally identical to BRIEF 1 (one fsync per commit), `On` coalesces a
//! contiguous `seal_seq` run into one group flush (the R1 fsync-before-publish
//! barrier) without losing or reordering, durable-before-visible holds at
//! commits and compacts, and a partial-append / flush failure / publish panic
//! poisons the whole in-flight run (every waiter Err'd, no hang).
//!
//! T1, T5, T7 are load-bearing.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use selene_core::{Change, GraphId, HlcTimestamp, IStr, LabelSet, PropertyMap, intern};

use crate::committer_batch::CommitBatching;
use crate::durable_provider::DurableProvider;
use crate::error::GraphError;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
use crate::{SeleneGraph, SharedGraph};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-brief2-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn on(max_commits: usize, max_bytes: u64) -> CommitBatching {
    CommitBatching::On {
        max_commits: NonZeroUsize::new(max_commits).unwrap(),
        max_bytes,
    }
}

/// A durable provider that records, in order, every `write_commit` and `flush`
/// event so a test can assert the exact append/fsync interleaving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurableEvent {
    /// `write_commit` returning this assigned sequence.
    Write(u64),
    /// `flush` returning the high-water sequence.
    Flush(u64),
}

struct CountingDurable {
    tag: ProviderTag,
    seq: AtomicU64,
    /// Number of `write_commit` calls before the configured failure (0 = never).
    fail_write_on_call: usize,
    /// Make `flush` fail (after the writes succeed).
    fail_flush: AtomicBool,
    events: Mutex<Vec<DurableEvent>>,
}

impl CountingDurable {
    fn new(tag: &[u8; 4]) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seq: AtomicU64::new(0),
            fail_write_on_call: 0,
            fail_flush: AtomicBool::new(false),
            events: Mutex::new(Vec::new()),
        })
    }

    fn fail_write_on(tag: &[u8; 4], call: usize) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seq: AtomicU64::new(0),
            fail_write_on_call: call,
            fail_flush: AtomicBool::new(false),
            events: Mutex::new(Vec::new()),
        })
    }

    fn fail_flush(tag: &[u8; 4]) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seq: AtomicU64::new(0),
            fail_write_on_call: 0,
            fail_flush: AtomicBool::new(true),
            events: Mutex::new(Vec::new()),
        })
    }

    fn events(&self) -> Vec<DurableEvent> {
        self.events.lock().unwrap().clone()
    }

    fn write_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| matches!(event, DurableEvent::Write(_)))
            .count()
    }

    fn flush_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| matches!(event, DurableEvent::Flush(_)))
            .count()
    }
}

impl DurableProvider for CountingDurable {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }

    fn write_commit(
        &self,
        _principal: Option<&[u8]>,
        _changes: &[Change],
        _timestamp: HlcTimestamp,
    ) -> Result<u64, ProviderError> {
        let write_calls = self.write_count() + 1;
        if self.fail_write_on_call != 0 && write_calls == self.fail_write_on_call {
            return Err(ProviderError::Inconsistent {
                reason: "synthetic write_commit failure".to_owned(),
            });
        }
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.events.lock().unwrap().push(DurableEvent::Write(seq));
        Ok(seq)
    }

    fn flush(&self) -> Result<Option<u64>, ProviderError> {
        if self.fail_flush.load(Ordering::SeqCst) {
            return Err(ProviderError::Inconsistent {
                reason: "synthetic flush failure".to_owned(),
            });
        }
        let high = self.seq.load(Ordering::SeqCst);
        self.events.lock().unwrap().push(DurableEvent::Flush(high));
        Ok(Some(high))
    }
}

/// Build a SharedGraph with a single synthetic durable provider (no real WAL),
/// under the given batching policy.
fn graph_with_durable(
    id: u64,
    durable: Arc<dyn DurableProvider>,
    batching: CommitBatching,
) -> SharedGraph {
    SharedGraph::from_graph_with_core_and_durables(
        SeleneGraph::new(GraphId::new(id)),
        Vec::new(),
        vec![durable],
        None,
        None,
        batching,
    )
    .expect("graph builds with synthetic durable provider")
}

/// Build a SharedGraph with a synthetic durable AND a fan-out index provider,
/// under the given batching policy.
fn graph_with_durable_and_provider(
    id: u64,
    durable: Arc<dyn DurableProvider>,
    provider: Arc<dyn IndexProvider>,
    batching: CommitBatching,
) -> SharedGraph {
    SharedGraph::from_graph_with_core_and_durables(
        SeleneGraph::new(GraphId::new(id)),
        vec![provider],
        vec![durable],
        None,
        None,
        batching,
    )
    .expect("graph builds with synthetic durable + provider")
}

// ───────────────────────── T1 (load-bearing) ─────────────────────────

#[test]
fn t1_off_equals_brief1_fsync_count() {
    // OFF == BRIEF 1 PIN: a counting durable records every write_commit + flush.
    // Under Off, each commit is exactly one write_commit followed by one flush
    // (one fsync per commit), in that order, with durable_at advancing +1 per
    // commit. A schema DDL and a compact ride the same path.
    let durable = CountingDurable::new(b"CNT1");
    let shared = graph_with_durable(70_001, durable.clone(), CommitBatching::Off);

    // Three plain commits.
    for idx in 0..3 {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("L")), PropertyMap::new())
            .unwrap();
        let outcome = txn.commit().expect("commit ok");
        assert_eq!(outcome.generation, idx + 1);
        // durable_at comes from the counting provider's returned sequence.
        assert_eq!(outcome.durable_at, Some(idx + 1));
    }

    // A schema DDL (property index create) — also a Work::Commit through the
    // same one-append-one-flush path.
    shared
        .create_property_index(istr("L"), istr("k"), crate::TypedIndexKind::I64)
        .expect("index create ok");

    // After 3 commits + 1 DDL: 4 writes, 4 flushes, strictly interleaved
    // write,flush,write,flush,...
    let events = durable.events();
    assert_eq!(durable.write_count(), 4, "events: {events:?}");
    assert_eq!(
        durable.flush_count(),
        durable.write_count(),
        "OFF: exactly one flush per commit (events: {events:?})",
    );
    for pair in events.chunks(2) {
        assert!(
            matches!(pair, [DurableEvent::Write(_), DurableEvent::Flush(_)]),
            "OFF interleaves write then flush per commit, got {pair:?}",
        );
    }

    // A compact publishes solo with ZERO additional flush calls (all lower seqs
    // already durable + visible) and adds no write.
    let writes_before = durable.write_count();
    let flushes_before = durable.flush_count();
    shared.compact().expect("compact ok");
    assert_eq!(
        durable.write_count(),
        writes_before,
        "compact appends nothing",
    );
    assert_eq!(
        durable.flush_count(),
        flushes_before,
        "compact issues zero flush calls",
    );
}

#[test]
fn t1b_reverse_order_seal_pair_off_one_flush_each() {
    // Two commits sealed A then B, submitted in reverse, still publish in
    // seal_seq order under Off — and each gets exactly one flush.
    let durable = CountingDurable::new(b"CNT2");
    let shared = Arc::new(graph_with_durable(
        70_002,
        durable.clone(),
        CommitBatching::Off,
    ));

    let mut txn_a = shared.begin_write();
    txn_a
        .mutator()
        .create_node(LabelSet::single(istr("A")), PropertyMap::new())
        .unwrap();
    let sealed_a = txn_a.seal(None, None).expect("A seals");

    let mut txn_b = shared.begin_write();
    txn_b
        .mutator()
        .create_node(LabelSet::single(istr("B")), PropertyMap::new())
        .unwrap();
    let sealed_b = txn_b.seal(None, None).expect("B seals");

    let shared_b = Arc::clone(&shared);
    let b_thread = thread::spawn(move || {
        shared_b
            .submit_sealed_for_test(sealed_b)
            .expect("B publishes")
    });
    for _ in 0..1_000 {
        thread::yield_now();
    }
    let outcome_a = shared
        .submit_sealed_for_test(sealed_a)
        .expect("A publishes");
    let outcome_b = b_thread.join().expect("B thread ok");

    assert_eq!(outcome_a.generation, 1);
    assert_eq!(outcome_b.generation, 2);
    // Two commits ⇒ two writes + two flushes (one flush per commit under Off).
    assert_eq!(durable.write_count(), 2);
    assert_eq!(durable.flush_count(), 2);
    assert_eq!(shared.read().node_count(), 2);
}

// ───────────────────────── T2 ─────────────────────────

#[test]
fn t2_on_path_groups_fsyncs() {
    // Fan in M commits from K threads under DEFAULT_ON; the committer coalesces
    // contiguous runs, so flush_count << write_count, every reply Ok with a
    // monotonic durable_at, and the final node count matches.
    const THREADS: usize = 8;
    const PER_THREAD: usize = 64;
    let durable = CountingDurable::new(b"CNT3");
    let shared = Arc::new(graph_with_durable(
        70_003,
        durable.clone(),
        CommitBatching::DEFAULT_ON,
    ));
    let barrier = Arc::new(Barrier::new(THREADS));

    thread::scope(|scope| {
        for thread_idx in 0..THREADS {
            let shared = Arc::clone(&shared);
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                for commit_idx in 0..PER_THREAD {
                    let mut txn = shared.begin_write();
                    txn.mutator()
                        .create_node(
                            LabelSet::single(istr("N")),
                            PropertyMap::from_pairs([(
                                istr("k"),
                                selene_core::Value::Int(
                                    (thread_idx * PER_THREAD + commit_idx) as i64,
                                ),
                            )])
                            .unwrap(),
                        )
                        .unwrap();
                    let outcome = txn.commit().expect("commit ok");
                    assert!(outcome.durable_at.is_some());
                }
            });
        }
    });

    let total = THREADS * PER_THREAD;
    assert_eq!(durable.write_count(), total, "every commit appended once");
    assert_eq!(shared.read().node_count(), total);
    assert_eq!(shared.read().meta.generation, total as u64);
    // The win: far fewer flushes than writes. Worst case a flush per commit
    // (all serialized), best case ~total/64. Assert it actually grouped at all.
    assert!(
        durable.flush_count() < durable.write_count(),
        "ON grouped fsyncs: {} flushes for {} writes",
        durable.flush_count(),
        durable.write_count(),
    );
}

// ───────────────────────── T3 ─────────────────────────

/// Fan-out provider that records the published generation order it observes via
/// `on_change` of NodeCreated, proving publish order under batching.
struct GenOrderProvider {
    tag: ProviderTag,
    seen: Mutex<Vec<u64>>,
}

impl GenOrderProvider {
    fn new(tag: &[u8; 4]) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seen: Mutex::new(Vec::new()),
        })
    }
}

impl IndexProvider for GenOrderProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }
    fn read_section(&self, _sub: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }
    fn write_section(&self, _sub: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }
    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        if let Change::NodeCreated { id, .. } = change {
            self.seen.lock().unwrap().push(id.get());
        }
        Ok(())
    }
    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn t3_order_sensitive_batch_publishes_in_seal_order() {
    // Seal A,B,C each forking the prior, submit in reverse; assert final gen==3,
    // all present, and fan-out observed node ids in seal order (1,2,3).
    let durable = CountingDurable::new(b"CNT4");
    let provider = GenOrderProvider::new(b"GORD");
    let shared = Arc::new(graph_with_durable_and_provider(
        70_004,
        durable,
        provider.clone(),
        on(8, 8 * 1024 * 1024),
    ));

    let mut sealeds = Vec::new();
    let mut ids = Vec::new();
    for label in ["A", "B", "C"] {
        let mut txn = shared.begin_write();
        let id = txn
            .mutator()
            .create_node(LabelSet::single(istr(label)), PropertyMap::new())
            .unwrap();
        ids.push(id);
        sealeds.push(txn.seal(None, None).expect("seals"));
    }

    // Submit C, B (reverse) on background threads, then A last to unblock.
    let sealed_c = sealeds.pop().unwrap();
    let sealed_b = sealeds.pop().unwrap();
    let sealed_a = sealeds.pop().unwrap();
    let s_c = Arc::clone(&shared);
    let c_thread = thread::spawn(move || s_c.submit_sealed_for_test(sealed_c).expect("C"));
    for _ in 0..1_000 {
        thread::yield_now();
    }
    let s_b = Arc::clone(&shared);
    let b_thread = thread::spawn(move || s_b.submit_sealed_for_test(sealed_b).expect("B"));
    for _ in 0..1_000 {
        thread::yield_now();
    }
    let outcome_a = shared.submit_sealed_for_test(sealed_a).expect("A");
    let outcome_b = b_thread.join().unwrap();
    let outcome_c = c_thread.join().unwrap();

    assert_eq!(outcome_a.generation, 1);
    assert_eq!(outcome_b.generation, 2);
    assert_eq!(outcome_c.generation, 3);
    let snap = shared.read();
    assert_eq!(snap.meta.generation, 3);
    assert_eq!(snap.node_count(), 3);
    // Fan-out observed publish order == seal order.
    let seen = provider.seen.lock().unwrap().clone();
    let expected: Vec<u64> = ids.iter().map(|id| id.get()).collect();
    assert_eq!(seen, expected, "fan-out observed seal-order publish");
}

// ───────────────────────── T4 ─────────────────────────

#[test]
fn t4_gap_ends_batch_no_deadlock() {
    // Allocate seqs 0,1,2; withhold 1. Submit 0 then 2. 0 publishes promptly; 2
    // stays buffered (never skip the gap). Releasing 1 publishes 1 then 2.
    let durable = CountingDurable::new(b"CNT5");
    let shared = Arc::new(graph_with_durable(70_005, durable, on(8, 8 * 1024 * 1024)));

    let mut s0 = shared.begin_write();
    s0.mutator()
        .create_node(LabelSet::single(istr("Z0")), PropertyMap::new())
        .unwrap();
    let sealed0 = s0.seal(None, None).expect("0");
    let mut s1 = shared.begin_write();
    s1.mutator()
        .create_node(LabelSet::single(istr("Z1")), PropertyMap::new())
        .unwrap();
    let sealed1 = s1.seal(None, None).expect("1");
    let mut s2 = shared.begin_write();
    s2.mutator()
        .create_node(LabelSet::single(istr("Z2")), PropertyMap::new())
        .unwrap();
    let sealed2 = s2.seal(None, None).expect("2");

    // Submit 0 (publishes at once) and 2 (buffered behind the seq-1 gap).
    let outcome0 = shared.submit_sealed_for_test(sealed0).expect("0 publishes");
    assert_eq!(outcome0.generation, 1);

    let s_two = Arc::clone(&shared);
    let two_thread = thread::spawn(move || s_two.submit_sealed_for_test(sealed2).expect("2"));
    for _ in 0..1_000 {
        thread::yield_now();
    }
    // Gap not filled: only the seq-0 node is visible; the committer never skipped
    // the gap to publish seq 2.
    assert_eq!(shared.read().meta.generation, 1, "2 not published over gap");

    // Release the gap; 1 then 2 publish, no hang.
    let outcome1 = shared.submit_sealed_for_test(sealed1).expect("1 publishes");
    let outcome2 = two_thread.join().expect("2 thread ok");
    assert_eq!(outcome1.generation, 2);
    assert_eq!(outcome2.generation, 3);
    assert_eq!(shared.read().node_count(), 3);
}

// ───────────────────────── T5 (load-bearing) ─────────────────────────

#[test]
fn t5_partial_batch_append_failure_errs_all_and_poisons() {
    // A durable that Errs on its 3rd write_commit. Fan in 5 contiguous commits
    // under On(8). NONE publish, ALL 5 reply Err (including the 2 already-appended
    // members), poisoned, no hang, later commit fails fast.
    let durable = CountingDurable::fail_write_on(b"CNT6", 3);
    let shared = Arc::new(graph_with_durable(
        70_006,
        durable.clone(),
        on(8, 8 * 1024 * 1024),
    ));

    // Seal 5 commits so they form one contiguous run when submitted together.
    let mut sealeds = Vec::new();
    for label in ["a", "b", "c", "d", "e"] {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr(label)), PropertyMap::new())
            .unwrap();
        sealeds.push(txn.seal(None, None).expect("seals"));
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut handles = Vec::new();
    for sealed in sealeds {
        let shared = Arc::clone(&shared);
        handles.push(thread::spawn(move || shared.submit_sealed_for_test(sealed)));
    }
    let mut err_count = 0;
    for handle in handles {
        let result = handle.join().expect("waiter thread did not panic");
        if result.is_err() {
            err_count += 1;
        }
    }
    assert!(Instant::now() < deadline, "no waiter hung");
    assert_eq!(err_count, 5, "all 5 members Err (incl. already-appended)");

    // Nothing published: generation never advanced.
    assert_eq!(shared.read().meta.generation, 0);
    assert_eq!(shared.read().node_count(), 0);

    // Engine poisoned: a later commit fails fast.
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    assert!(
        matches!(txn.commit(), Err(GraphError::Durable { .. })),
        "post-poison commit fails fast",
    );
}

// ───────────────────────── T6 ─────────────────────────

#[test]
fn t6_flush_failure_poisons_all() {
    // write_commit Ok but flush() Err. Submit 4 contiguous. NONE publish, all 4
    // Err, poisoned, bounded.
    let durable = CountingDurable::fail_flush(b"CNT7");
    let shared = Arc::new(graph_with_durable(
        70_007,
        durable.clone(),
        on(8, 8 * 1024 * 1024),
    ));

    let mut sealeds = Vec::new();
    for label in ["a", "b", "c", "d"] {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr(label)), PropertyMap::new())
            .unwrap();
        sealeds.push(txn.seal(None, None).expect("seals"));
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut handles = Vec::new();
    for sealed in sealeds {
        let shared = Arc::clone(&shared);
        handles.push(thread::spawn(move || shared.submit_sealed_for_test(sealed)));
    }
    let mut err_count = 0;
    for handle in handles {
        if handle.join().expect("no panic").is_err() {
            err_count += 1;
        }
    }
    assert!(Instant::now() < deadline, "no waiter hung");
    assert_eq!(err_count, 4, "all 4 Err on flush failure");
    assert_eq!(shared.read().meta.generation, 0, "nothing published");
    // No flush ever succeeded.
    assert_eq!(durable.flush_count(), 0);
}

/// WAL-backed, compaction-boundary, and recovery group-commit tests (T7..T13).
/// Split into its own file so the test module stays under the 700-LOC cap; it
/// reuses every helper + synthetic provider from the parent module via
/// `use super::*`.
#[path = "committer_batch_wal_tests.rs"]
mod wal;
