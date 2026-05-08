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
    /// published. Provider errors are logged and do not fail the graph commit.
    pub fn commit_with_principal(
        mut self,
        principal: Option<Arc<[u8]>>,
    ) -> GraphResult<CommitOutcome> {
        self.working.meta.generation = self
            .working
            .meta
            .generation
            .checked_add(1)
            .expect("graph generation exhausted");
        self.working.meta.next_node_id = self.allocator.peek_next_node();
        self.working.meta.next_edge_id = self.allocator.peek_next_edge();

        let generation = self.working.meta.generation;
        let next_node_id = self.working.meta.next_node_id;
        let next_edge_id = self.working.meta.next_edge_id;
        let published = self.working.clone();
        *self.guard = published.clone();
        self.snapshot.store(Arc::new(published));

        for change in &self.changes {
            for provider in &self.providers {
                if let Err(error) = provider.on_change(change) {
                    tracing::error!(
                        provider_tag = %provider.provider_tag(),
                        error = %error,
                        ?change,
                        "index provider on_change failed after graph commit; continuing"
                    );
                }
            }
        }

        Ok(CommitOutcome {
            generation,
            changes: std::mem::take(&mut self.changes),
            principal,
            next_node_id,
            next_edge_id,
        })
    }

    /// Roll back graph changes and release the write lock.
    pub fn rollback(self) {}
}

impl Drop for WriteTxn<'_> {
    fn drop(&mut self) {}
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
