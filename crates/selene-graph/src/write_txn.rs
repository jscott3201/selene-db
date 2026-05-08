//! Write transaction RAII handle per spec 03 sections 4 and 6.

use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::{MutexGuard, RwLockWriteGuard};
use selene_core::{Change, Origin};

use crate::error::GraphResult;
use crate::graph::SeleneGraph;
use crate::id_allocator::IdAllocator;
use crate::index_provider::IndexProvider;
use crate::mutator::Mutator;

/// Result metadata returned after a successful commit.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitOutcome {
    /// Published graph generation.
    pub generation: u64,
    /// Changes produced by the mutation funnel.
    pub changes: Vec<Change>,
    /// Opaque caller-supplied principal bytes for future WAL headers.
    pub principal: Option<Arc<[u8]>>,
    /// Next node ID after commit.
    pub next_node_id: u64,
    /// Next edge ID after commit.
    pub next_edge_id: u64,
}

/// RAII owner of the single graph write lock.
pub struct WriteTxn<'g> {
    pub(crate) guard: RwLockWriteGuard<'g, SeleneGraph>,
    pub(crate) snapshot: Arc<ArcSwap<SeleneGraph>>,
    pub(crate) working: SeleneGraph,
    pub(crate) allocator: MutexGuard<'g, IdAllocator>,
    pub(crate) providers: Vec<Arc<dyn IndexProvider>>,
    pub(crate) changes: Vec<Change>,
}

impl<'g> WriteTxn<'g> {
    pub(crate) fn new(
        guard: RwLockWriteGuard<'g, SeleneGraph>,
        snapshot: Arc<ArcSwap<SeleneGraph>>,
        allocator: MutexGuard<'g, IdAllocator>,
        providers: Vec<Arc<dyn IndexProvider>>,
    ) -> Self {
        let working = guard.clone();
        Self {
            guard,
            snapshot,
            working,
            allocator,
            providers,
            changes: Vec::new(),
        }
    }

    /// Borrow a mutator tied to this transaction.
    #[must_use]
    pub fn mutator(&mut self) -> Mutator<'_, 'g> {
        Mutator::new(self, Origin::Local)
    }

    /// Commit without caller principal bytes.
    pub fn commit(self) -> GraphResult<CommitOutcome> {
        self.commit_with_principal(None)
    }

    /// Commit with optional caller-owned principal bytes for D12 audit replay.
    ///
    /// Registered index providers are notified after the new graph snapshot is
    /// published, **with the write lock and allocator mutex still held**, so
    /// that two concurrent commits cannot interleave their `on_change`
    /// callbacks (the per-graph serialization contract from
    /// `_spec/06-index-provider-protocol.md`). Same-thread re-entrant
    /// provider calls into `SharedGraph::begin_write()` are detected via a
    /// thread-local fanout counter and panic with a clear message; the
    /// outer `std::panic::catch_unwind` in `notify_providers` catches those
    /// panics (along with provider-internal panics and returned errors) so
    /// a single misbehaving provider can never abort the writer thread.
    /// Cross-thread re-entry (a provider waiting on a spawned worker that
    /// calls `begin_write`) is documented misuse — see `reentry.rs` and
    /// the `IndexProvider` rustdoc.
    pub fn commit_with_principal(self, principal: Option<Arc<[u8]>>) -> GraphResult<CommitOutcome> {
        let WriteTxn {
            mut guard,
            snapshot,
            mut working,
            allocator,
            providers,
            changes,
        } = self;

        working.meta.generation = working
            .meta
            .generation
            .checked_add(1)
            .expect("graph generation exhausted");
        working.meta.next_node_id = allocator.peek_next_node();
        working.meta.next_edge_id = allocator.peek_next_edge();

        let generation = working.meta.generation;
        let next_node_id = working.meta.next_node_id;
        let next_edge_id = working.meta.next_edge_id;

        if let Some(type_def) = working.meta.bound_type.as_deref() {
            for change in &changes {
                crate::type_validator::validate_change(change, &working, type_def)?;
            }
        }

        let published = working.clone();
        *guard = published.clone();
        snapshot.store(Arc::new(published));

        // Hold guard + allocator across fanout. The thread-local fanout
        // guard increments a counter that `SharedGraph::begin_write` checks
        // before attempting any locking, so same-thread re-entrant writes
        // from inside `on_change` panic before reaching this lock — no
        // deadlock, and commit serialization is preserved. Concurrent
        // writers from other threads queue normally on the write lock.
        {
            let _fanout_guard = crate::reentry::FanoutGuard::enter();
            notify_providers(&providers, &changes);
        }

        // Releasing guard + allocator AFTER fanout. Other writers can now
        // start their own commits; their fanouts run after this one in
        // strict serial order.
        drop(guard);
        drop(allocator);

        Ok(CommitOutcome {
            generation,
            changes,
            principal,
            next_node_id,
            next_edge_id,
        })
    }

    /// Roll back graph changes and release the write lock.
    pub fn rollback(self) {}
}

// No explicit `Drop` impl — the standard field-drop order releases the
// `RwLockWriteGuard` (and the allocator `MutexGuard`), which is the actual
// rollback semantic. An explicit `Drop` impl would also block the
// destructuring move in `commit_with_principal`.

/// Fan out committed changes to every registered provider, swallowing
/// returned errors and panics so a misbehaving provider can never abort or
/// crash the writer thread after the snapshot has already published.
///
/// Each iteration first reads the provider's tag through its own unwind
/// boundary so a panic in `provider_tag()` is logged with the sentinel
/// `<unknown>` tag and the provider is **skipped for this change** —
/// matching the original "single combined unwind" behavior where a tag
/// panic short-circuited `on_change`. When the tag is read successfully it
/// is reused in both the error-return and panic branches of `on_change` so
/// operators can attribute failures to the faulty provider.
fn notify_providers(providers: &[Arc<dyn IndexProvider>], changes: &[Change]) {
    for change in changes {
        for provider in providers {
            // First boundary: cache the provider tag for logging. If
            // `provider_tag()` itself panics, log with the sentinel tag and
            // skip `on_change` — the provider is in an inconsistent state
            // and we should not invoke further side effects on it.
            let tag = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                provider.provider_tag()
            })) {
                Ok(tag) => tag,
                Err(payload) => {
                    let payload = describe_panic_payload(&payload);
                    tracing::error!(
                        provider_tag = %SENTINEL_PROVIDER_TAG,
                        ?change,
                        payload = %payload,
                        "index provider provider_tag() panicked after graph commit; \
                         skipping on_change for this change",
                    );
                    continue;
                }
            };

            // Second boundary: invoke `on_change`. The cached tag is
            // available for both the error-return and panic branches.
            // AssertUnwindSafe: provider interior state may be left
            // half-updated by a panic. The engine's contract is that the
            // graph commit succeeded; provider drift is logged but not
            // catastrophic.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                provider.on_change(change)
            }));
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(
                        provider_tag = %tag,
                        error = %error,
                        ?change,
                        "index provider on_change failed after graph commit; continuing",
                    );
                }
                Err(panic_payload) => {
                    let payload = describe_panic_payload(&panic_payload);
                    tracing::error!(
                        provider_tag = %tag,
                        ?change,
                        payload = %payload,
                        "index provider on_change panicked after graph commit; continuing",
                    );
                }
            }
        }
    }
}

/// Sentinel value emitted on the `provider_tag` field when the provider's
/// own `provider_tag()` method panicked, so log filters keyed on the field
/// name still match.
const SENTINEL_PROVIDER_TAG: &str = "<unknown>";

fn describe_panic_payload(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use parking_lot::Mutex;
    use selene_core::{Change, GraphId, LabelSet, NodeId, PropertyMap, Value, intern};

    use crate::{IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};

    struct RecordingProvider {
        tag: ProviderTag,
        seen: Arc<Mutex<Vec<(ProviderTag, Change)>>>,
        fail: bool,
    }

    impl RecordingProvider {
        fn new(tag: ProviderTag, seen: Arc<Mutex<Vec<(ProviderTag, Change)>>>) -> Self {
            Self {
                tag,
                seen,
                fail: false,
            }
        }

        fn failing(tag: ProviderTag, seen: Arc<Mutex<Vec<(ProviderTag, Change)>>>) -> Self {
            Self {
                tag,
                seen,
                fail: true,
            }
        }
    }

    impl IndexProvider for RecordingProvider {
        fn provider_tag(&self) -> ProviderTag {
            self.tag
        }

        fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
            Ok(())
        }

        fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
            Ok(Vec::new())
        }

        fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
            self.seen.lock().push((self.tag, change.clone()));
            if self.fail {
                Err(ProviderError::Inconsistent {
                    reason: "synthetic provider failure".to_owned(),
                })
            } else {
                Ok(())
            }
        }

        fn declared_sub_tags(&self) -> &[SubTag] {
            &[]
        }
    }

    #[test]
    fn commit_publishes_new_snapshot() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let node_id = {
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok")
        };
        let outcome = txn.commit().unwrap();
        assert_eq!(outcome.generation, 1);
        assert!(shared.read().is_node_alive(node_id));
    }

    #[test]
    fn rollback_does_not_publish() {
        let shared = SharedGraph::new(GraphId::new(1));
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok");
            txn.rollback();
        }
        assert_eq!(shared.read().node_count(), 0);
    }

    #[test]
    fn aborted_tx_ids_become_permanent_holes() {
        let shared = SharedGraph::new(GraphId::new(1));
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            assert_eq!(
                mutator
                    .create_node(LabelSet::new(), PropertyMap::new())
                    .expect("create_node ok"),
                NodeId::new(1)
            );
        }
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            assert_eq!(
                mutator
                    .create_node(LabelSet::new(), PropertyMap::new())
                    .expect("create_node ok"),
                NodeId::new(2)
            );
            txn.commit().unwrap();
        }
        let snapshot = shared.read();
        assert!(!snapshot.is_node_alive(NodeId::new(1)));
        assert!(snapshot.is_node_alive(NodeId::new(2)));
        assert_eq!(snapshot.meta.next_node_id, 3);
    }

    #[test]
    fn commit_with_principal_carries_principal_to_outcome() {
        let shared = SharedGraph::new(GraphId::new(1));
        let principal = Arc::from([1_u8, 2, 3]);
        let outcome = shared
            .begin_write()
            .commit_with_principal(Some(Arc::clone(&principal)))
            .unwrap();
        assert_eq!(outcome.principal.as_deref(), Some(&principal[..]));
    }

    #[test]
    fn commit_returns_changes_in_order() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let label = intern("txn.node").unwrap();
        {
            let mut mutator = txn.mutator();
            let id = mutator
                .create_node(LabelSet::single(label), PropertyMap::new())
                .expect("create_node ok");
            let prop = intern("txn.prop").unwrap();
            let diff = selene_core::PropertyDiff::new([(prop, Value::Int(1))], []).unwrap();
            mutator
                .update_node(id, selene_core::LabelDiff::new([], []).unwrap(), diff)
                .unwrap();
        }
        let outcome = txn.commit().unwrap();
        assert!(matches!(outcome.changes[0], Change::NodeCreated { .. }));
        assert!(matches!(outcome.changes[1], Change::NodeUpdated { .. }));
    }

    #[test]
    fn commit_calls_on_change_for_every_change_in_order() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shared = SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(RecordingProvider::new(
                ProviderTag(*b"RECD"),
                Arc::clone(&seen),
            )))
            .build()
            .unwrap();
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            let id = mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok");
            mutator.delete_node(id).unwrap();
        }
        txn.commit().unwrap();
        let seen = seen.lock();
        assert!(matches!(seen[0].1, Change::NodeCreated { .. }));
        assert!(matches!(seen[1].1, Change::NodeDeleted { .. }));
    }

    #[test]
    fn commit_calls_each_provider_in_registration_order() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shared = SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(RecordingProvider::new(
                ProviderTag(*b"ONE1"),
                Arc::clone(&seen),
            )))
            .with_provider(Arc::new(RecordingProvider::new(
                ProviderTag(*b"TWO2"),
                Arc::clone(&seen),
            )))
            .build()
            .unwrap();
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok");
        }
        txn.commit().unwrap();
        let tags = seen.lock().iter().map(|(tag, _)| *tag).collect::<Vec<_>>();
        assert_eq!(tags, vec![ProviderTag(*b"ONE1"), ProviderTag(*b"TWO2")]);
    }

    #[test]
    fn provider_error_does_not_fail_commit() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shared = SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(RecordingProvider::failing(
                ProviderTag(*b"FAIL"),
                Arc::clone(&seen),
            )))
            .build()
            .unwrap();
        let mut txn = shared.begin_write();
        let id = {
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok")
        };
        let outcome = txn.commit().unwrap();
        assert!(shared.read().is_node_alive(id));
        assert_eq!(outcome.changes.len(), 1);
        assert_eq!(seen.lock().len(), 1);
    }

    #[test]
    fn rollback_does_not_call_on_change() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shared = SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(RecordingProvider::new(
                ProviderTag(*b"ROLL"),
                Arc::clone(&seen),
            )))
            .build()
            .unwrap();
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok");
            txn.rollback();
        }
        assert!(seen.lock().is_empty());
    }

    #[test]
    fn concurrent_writers_serialize() {
        let shared = Arc::new(SharedGraph::new(GraphId::new(1)));
        thread::scope(|scope| {
            for _ in 0..4 {
                let shared = Arc::clone(&shared);
                scope.spawn(move || {
                    let mut txn = shared.begin_write();
                    {
                        let mut mutator = txn.mutator();
                        let _ = mutator.create_node(LabelSet::new(), PropertyMap::new());
                    }
                    txn.commit().unwrap();
                });
            }
        });
        assert_eq!(shared.read().node_count(), 4);
    }

    /// A provider whose `on_change` tries to start a nested write via
    /// `SharedGraph::begin_write()`. With the v1.0 contract this is misuse
    /// and must panic — caught by `notify_providers` so the outer commit
    /// completes regardless. The `chained_count` increments only if the
    /// nested write completes; it must stay at 0.
    struct ReentrantProvider {
        tag: ProviderTag,
        shared: Mutex<Option<Arc<SharedGraph>>>,
        chained_count: Arc<Mutex<usize>>,
    }

    impl ReentrantProvider {
        fn new(tag: ProviderTag) -> Self {
            Self {
                tag,
                shared: Mutex::new(None),
                chained_count: Arc::new(Mutex::new(0)),
            }
        }

        fn install_shared(&self, shared: Arc<SharedGraph>) {
            *self.shared.lock() = Some(shared);
        }
    }

    impl IndexProvider for ReentrantProvider {
        fn provider_tag(&self) -> ProviderTag {
            self.tag
        }

        fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
            Ok(())
        }

        fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
            Ok(Vec::new())
        }

        fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
            // begin_write() panics on this thread because the FanoutGuard is
            // active. The panic unwinds out of on_change before reaching the
            // chained_count increment.
            let shared = self.shared.lock().take();
            if let Some(shared) = shared {
                let txn = shared.begin_write();
                let _ = txn.commit();
                *self.chained_count.lock() += 1;
            }
            Ok(())
        }

        fn declared_sub_tags(&self) -> &[SubTag] {
            &[]
        }
    }

    #[test]
    fn begin_write_inside_provider_callback_panics_and_is_caught() {
        let provider = Arc::new(ReentrantProvider::new(ProviderTag(*b"REEN")));
        let chained_count = Arc::clone(&provider.chained_count);
        let shared = Arc::new(
            SharedGraph::builder(GraphId::new(1))
                .with_provider(Arc::clone(&provider) as Arc<dyn IndexProvider>)
                .build()
                .unwrap(),
        );
        provider.install_shared(Arc::clone(&shared));

        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok");
        }
        // Outer commit completes despite the provider's misuse — the panic
        // raised by begin_write inside on_change is caught by
        // notify_providers' catch_unwind boundary.
        let outcome = txn.commit().unwrap();
        assert_eq!(outcome.changes.len(), 1);
        // The increment after begin_write/commit was never reached.
        assert_eq!(
            *chained_count.lock(),
            0,
            "provider's chained mutation must not have completed"
        );
        // After the outer commit returns, the graph's fanout flag is clear
        // and a fresh begin_write succeeds normally.
        let txn = shared.begin_write();
        txn.rollback();
    }

    /// A provider whose `on_change` panics. Used to verify the engine catches
    /// the unwind and continues serving subsequent providers.
    struct PanickingProvider {
        tag: ProviderTag,
    }

    impl IndexProvider for PanickingProvider {
        fn provider_tag(&self) -> ProviderTag {
            self.tag
        }

        fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
            Ok(())
        }

        fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
            Ok(Vec::new())
        }

        fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
            panic!("synthetic provider panic");
        }

        fn declared_sub_tags(&self) -> &[SubTag] {
            &[]
        }
    }

    #[test]
    fn provider_panic_does_not_crash_commit_or_block_other_providers() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shared = SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(PanickingProvider {
                tag: ProviderTag(*b"PANC"),
            }))
            .with_provider(Arc::new(RecordingProvider::new(
                ProviderTag(*b"AFTR"),
                Arc::clone(&seen),
            )))
            .build()
            .unwrap();
        let mut txn = shared.begin_write();
        let id = {
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok")
        };
        let outcome = txn.commit().unwrap();
        assert!(shared.read().is_node_alive(id));
        assert_eq!(outcome.changes.len(), 1);
        // The provider AFTER the panicking one still received the change.
        assert_eq!(seen.lock().len(), 1);
    }

    /// A provider whose `on_change` blocks for a brief window so a second
    /// writer thread can observe that it queues normally on the write lock
    /// without panicking — the explicit non-regression for the round-4 P1.
    struct SlowProvider {
        tag: ProviderTag,
        hold: std::time::Duration,
    }

    impl IndexProvider for SlowProvider {
        fn provider_tag(&self) -> ProviderTag {
            self.tag
        }

        fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
            Ok(())
        }

        fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
            Ok(Vec::new())
        }

        fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
            std::thread::sleep(self.hold);
            Ok(())
        }

        fn declared_sub_tags(&self) -> &[SubTag] {
            &[]
        }
    }

    #[test]
    fn concurrent_writer_does_not_panic_during_other_commits_fanout() {
        // Regression test for the round-4 P1: the previous design had
        // begin_write panic on every concurrent writer while another
        // commit's fanout was running. Concurrent writers must queue on
        // the write lock and proceed normally instead.
        let shared = Arc::new(
            SharedGraph::builder(GraphId::new(1))
                .with_provider(Arc::new(SlowProvider {
                    tag: ProviderTag(*b"SLOW"),
                    hold: std::time::Duration::from_millis(40),
                }))
                .build()
                .unwrap(),
        );

        let writer_a = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let mut txn = shared.begin_write();
                {
                    let mut mutator = txn.mutator();
                    mutator
                        .create_node(LabelSet::new(), PropertyMap::new())
                        .expect("create_node ok");
                }
                txn.commit().unwrap();
            })
        };

        // Give writer A a head start so it owns the lock and is inside the
        // SlowProvider's on_change.
        thread::sleep(std::time::Duration::from_millis(10));

        // Writer B begins a write while A's fanout is running. It must
        // queue on the write lock, NOT panic.
        let writer_b = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let mut txn = shared.begin_write();
                {
                    let mut mutator = txn.mutator();
                    mutator
                        .create_node(LabelSet::new(), PropertyMap::new())
                        .expect("create_node ok");
                }
                txn.commit().unwrap();
            })
        };

        writer_a.join().expect("writer A finished without panic");
        writer_b.join().expect("writer B finished without panic");
        assert_eq!(shared.read().node_count(), 2);
    }

    /// A provider whose `provider_tag()` panics only after the build-time
    /// uniqueness check has already run successfully. Used to verify the
    /// engine short-circuits `on_change` for that provider rather than
    /// calling it after a fanout-time tag panic.
    struct ConditionallyTagPanickingProvider {
        tag: ProviderTag,
        panic_during_fanout: Arc<std::sync::atomic::AtomicBool>,
        on_change_called: Arc<Mutex<bool>>,
    }

    impl IndexProvider for ConditionallyTagPanickingProvider {
        fn provider_tag(&self) -> ProviderTag {
            if self
                .panic_during_fanout
                .load(std::sync::atomic::Ordering::Acquire)
            {
                panic!("synthetic provider_tag() panic during fanout");
            }
            self.tag
        }

        fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
            Ok(())
        }

        fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
            Ok(Vec::new())
        }

        fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
            *self.on_change_called.lock() = true;
            Ok(())
        }

        fn declared_sub_tags(&self) -> &[SubTag] {
            &[]
        }
    }

    #[test]
    fn provider_tag_panic_short_circuits_on_change_for_that_provider() {
        let panic_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let on_change_called = Arc::new(Mutex::new(false));
        let other_seen = Arc::new(Mutex::new(Vec::new()));
        let shared = SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(ConditionallyTagPanickingProvider {
                tag: ProviderTag(*b"TPNC"),
                panic_during_fanout: Arc::clone(&panic_flag),
                on_change_called: Arc::clone(&on_change_called),
            }))
            .with_provider(Arc::new(RecordingProvider::new(
                ProviderTag(*b"OTHR"),
                Arc::clone(&other_seen),
            )))
            .build()
            .unwrap();

        // Arm the panic only after the build-time uniqueness check has run.
        panic_flag.store(true, std::sync::atomic::Ordering::Release);

        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok");
        }
        txn.commit().unwrap();
        assert!(
            !*on_change_called.lock(),
            "on_change must not run after provider_tag() panicked",
        );
        // The non-panicking provider after the panicking one still ran.
        assert_eq!(other_seen.lock().len(), 1);
    }

    #[test]
    #[cfg(not(miri))]
    fn concurrent_writers_notify_provider_for_every_change() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::new(
            SharedGraph::builder(GraphId::new(1))
                .with_provider(Arc::new(RecordingProvider::new(
                    ProviderTag(*b"CNCR"),
                    Arc::clone(&seen),
                )))
                .build()
                .unwrap(),
        );
        let nodes_per_thread = 64;
        thread::scope(|scope| {
            for _ in 0..4 {
                let shared = Arc::clone(&shared);
                scope.spawn(move || {
                    let mut txn = shared.begin_write();
                    {
                        let mut mutator = txn.mutator();
                        for _ in 0..nodes_per_thread {
                            mutator
                                .create_node(LabelSet::new(), PropertyMap::new())
                                .expect("create_node ok");
                        }
                    }
                    txn.commit().unwrap();
                });
            }
        });
        assert_eq!(shared.read().node_count(), 4 * nodes_per_thread);
        assert_eq!(seen.lock().len(), 4 * nodes_per_thread);
    }
}
