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
    /// published AND after the graph write lock is released. Provider errors
    /// (returned `Err`) and provider panics are both logged and do not fail
    /// the graph commit. Releasing the write lock before fanout prevents a
    /// re-entrant provider (one that calls `begin_write()` from `on_change`)
    /// from deadlocking on the same lock.
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

        let published = working.clone();
        *guard = published.clone();
        snapshot.store(Arc::new(published));

        // CRITICAL: release the write lock and the allocator mutex BEFORE
        // calling provider callbacks. A provider that re-enters via
        // `SharedGraph::begin_write()` would otherwise deadlock against the
        // very lock we hold here.
        drop(guard);
        drop(allocator);

        notify_providers(&providers, &changes);

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
fn notify_providers(providers: &[Arc<dyn IndexProvider>], changes: &[Change]) {
    for change in changes {
        for provider in providers {
            // AssertUnwindSafe: we don't care if the provider's interior state
            // is left half-updated by a panic — the engine's contract is that
            // the graph commit succeeded; provider state may drift, and a
            // future hardening pass can add a strict-mode knob if needed.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                provider.on_change(change)
            }));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(
                        provider_tag = %provider.provider_tag(),
                        error = %error,
                        ?change,
                        "index provider on_change failed after graph commit; continuing",
                    );
                }
                Err(panic_payload) => {
                    let payload = describe_panic_payload(&panic_payload);
                    tracing::error!(
                        provider_tag = %provider.provider_tag(),
                        ?change,
                        payload = %payload,
                        "index provider on_change panicked after graph commit; continuing",
                    );
                }
            }
        }
    }
}

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

    /// A provider that calls `begin_write()` from within its `on_change` to
    /// chain another transaction. Used to verify the commit path does NOT
    /// hold the write lock while invoking provider callbacks.
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
            // Take the shared graph reference once; chained on_change calls
            // would otherwise recurse infinitely.
            let shared = self.shared.lock().take();
            if let Some(shared) = shared {
                // This would deadlock if the outer commit still held the
                // write lock when calling on_change.
                let txn = shared.begin_write();
                txn.commit().expect("chained commit ok");
                *self.chained_count.lock() += 1;
            }
            Ok(())
        }

        fn declared_sub_tags(&self) -> &[SubTag] {
            &[]
        }
    }

    #[test]
    fn provider_can_re_enter_begin_write_without_deadlock() {
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
        // If the write lock were still held during provider fanout this
        // commit would deadlock the test harness.
        txn.commit().unwrap();
        assert_eq!(
            *chained_count.lock(),
            1,
            "provider chained one inner commit"
        );
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
