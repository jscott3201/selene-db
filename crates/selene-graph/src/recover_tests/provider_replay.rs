//! Recovery tests for `recover_with_providers` (index-provider WAL replay).

use std::sync::Arc;

use parking_lot::Mutex;
use selene_core::{Change, GraphId, NodeId};

use super::super::set_before_shared_construction_hook;
use super::{append_wal, node_created, temp_dir, write_snapshot};
use crate::{GraphError, IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};

const LIFECYCLE_SUB_TAGS: &[SubTag] = &[SubTag(*b"LIFE")];

struct NoopIndexProvider {
    tag: ProviderTag,
}

impl NoopIndexProvider {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl IndexProvider for NoopIndexProvider {
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
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn recover_with_providers_replays_wal() {
    let dir = temp_dir("provider-replay");
    append_wal(&dir, 0, &[node_created(1)]);
    let tag = ProviderTag(*b"NOOP");
    let provider: Arc<dyn IndexProvider> = Arc::new(NoopIndexProvider::new(tag));

    let recovered =
        SharedGraph::recover_with_providers(&dir, GraphId::new(7), vec![provider]).unwrap();

    assert!(recovered.read().is_node_alive(NodeId::new(1)));
    let _ = std::fs::remove_dir_all(dir);
}

/// An index provider whose `on_change` always errors, to prove WAL-replay
/// provider errors propagate out of `recover_with_providers` (GRAPH-39).
struct FailingIndexProvider {
    tag: ProviderTag,
}

impl FailingIndexProvider {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl IndexProvider for FailingIndexProvider {
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
        Err(ProviderError::Inconsistent {
            reason: "synthetic index-provider on_change failure".to_owned(),
        })
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn recover_with_providers_propagates_index_on_change_error() {
    // GRAPH-39: WAL replay drives every registered provider's `on_change` (via the
    // `IndexRecoveryProvider` wrapper) for each replayed entry. An index provider
    // that errors there must abort recovery with the boxed error surfaced — not be
    // silently swallowed (which would leave the rebuilt index inconsistent with
    // the recovered graph). Only the always-Ok Noop provider was exercised before.
    let dir = temp_dir("provider-replay-fail");
    append_wal(&dir, 0, &[node_created(1)]);
    let provider: Arc<dyn IndexProvider> =
        Arc::new(FailingIndexProvider::new(ProviderTag(*b"FAIL")));

    let err = match SharedGraph::recover_with_providers(&dir, GraphId::new(7), vec![provider]) {
        Ok(_) => panic!("recovery must fail when an index provider's on_change errors"),
        Err(error) => error,
    };
    let crate::GraphError::Persist(selene_persist::PersistError::ProviderFailed {
        provider,
        source,
        ..
    }) = &err
    else {
        panic!("expected PersistError::ProviderFailed, got {err:?}");
    };
    assert_eq!(
        *provider, *b"FAIL",
        "the failing provider's tag is surfaced"
    );
    assert!(
        format!("{source}").contains("synthetic index-provider on_change failure"),
        "the boxed provider error must surface verbatim, got: {source}",
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A provider whose per-change callback succeeds but whose batch callback fails
/// when recovery replays a persisted declarative reset. This pins the WAL
/// recovery seam that delivers commit batches through `on_changes`.
struct FailingBatchIndexProvider {
    tag: ProviderTag,
}

impl FailingBatchIndexProvider {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl IndexProvider for FailingBatchIndexProvider {
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
        Ok(())
    }

    fn handles_change_batches(&self) -> bool {
        true
    }

    fn on_changes(&self, changes: &[Change]) -> Result<(), ProviderError> {
        if changes
            .iter()
            .any(|change| matches!(change, Change::GraphReset {}))
        {
            return Err(ProviderError::Inconsistent {
                reason: format!(
                    "synthetic index-provider on_changes failure after {} replayed changes",
                    changes.len()
                ),
            });
        }
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn recover_with_providers_propagates_index_on_changes_error_for_reset() {
    let dir = temp_dir("provider-replay-batch-fail");
    append_wal(&dir, 0, &[node_created(1), Change::GraphReset {}]);
    let provider: Arc<dyn IndexProvider> =
        Arc::new(FailingBatchIndexProvider::new(ProviderTag(*b"BFL1")));

    let err = match SharedGraph::recover_with_providers(&dir, GraphId::new(7), vec![provider]) {
        Ok(_) => panic!("recovery must fail when an index provider's on_changes errors"),
        Err(error) => error,
    };
    let crate::GraphError::Persist(selene_persist::PersistError::ProviderFailed {
        provider,
        source,
        ..
    }) = &err
    else {
        panic!("expected PersistError::ProviderFailed, got {err:?}");
    };
    assert_eq!(
        *provider, *b"BFL1",
        "the failing provider's tag is surfaced"
    );
    let source = format!("{source}");
    assert!(
        source.contains("synthetic index-provider on_changes failure"),
        "the boxed provider error must surface verbatim, got: {source}",
    );
    assert!(
        source.contains("2 replayed changes"),
        "the provider must see the whole WAL commit batch, got: {source}",
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[derive(Default)]
struct AttachmentState {
    reserved: bool,
    attached: bool,
    aborts: usize,
}

struct LifecycleProvider {
    tag: ProviderTag,
    events: Arc<Mutex<Vec<String>>>,
    state: Mutex<AttachmentState>,
    fail_reserve: bool,
    fail_change: bool,
    fail_prepare: bool,
    fail_rebuild: bool,
    declares_state: bool,
}

impl LifecycleProvider {
    fn new(tag: [u8; 4], events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            tag: ProviderTag(tag),
            events,
            state: Mutex::new(AttachmentState::default()),
            fail_reserve: false,
            fail_change: false,
            fail_prepare: false,
            fail_rebuild: false,
            declares_state: false,
        }
    }

    fn event(&self, phase: &str) {
        self.events.lock().push(format!("{}:{phase}", self.tag));
    }
}

impl IndexProvider for LifecycleProvider {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        assert!(self.state.lock().reserved);
        self.event("read");
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        assert!(self.state.lock().reserved);
        self.event("change");
        if self.fail_change {
            return Err(ProviderError::Inconsistent {
                reason: format!("{} callback failpoint", self.tag),
            });
        }
        Ok(())
    }

    fn reserve_recovery_attachment(&self) -> Result<(), ProviderError> {
        self.event("reserve");
        if self.fail_reserve {
            return Err(ProviderError::Inconsistent {
                reason: format!("{} reserve failpoint", self.tag),
            });
        }
        self.state.lock().reserved = true;
        Ok(())
    }

    fn prepare_recovery_attachment(
        &self,
        _graph: &crate::SeleneGraph,
    ) -> Result<(), ProviderError> {
        assert!(self.state.lock().reserved);
        self.event("prepare");
        if self.fail_prepare {
            return Err(ProviderError::Inconsistent {
                reason: format!("{} prepare failpoint", self.tag),
            });
        }
        Ok(())
    }

    fn rebuild_from_graph(&self, _graph: &crate::SeleneGraph) -> Result<(), ProviderError> {
        assert!(self.state.lock().reserved);
        self.event("rebuild");
        if self.fail_rebuild {
            return Err(ProviderError::Inconsistent {
                reason: format!("{} rebuild failpoint", self.tag),
            });
        }
        Ok(())
    }

    fn commit_recovery_attachment(&self) {
        self.event("commit");
        let mut state = self.state.lock();
        assert!(state.reserved);
        state.reserved = false;
        state.attached = true;
    }

    fn abort_recovery_attachment(&self) {
        self.event("abort");
        let mut state = self.state.lock();
        state.reserved = false;
        state.aborts += 1;
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        if self.declares_state {
            LIFECYCLE_SUB_TAGS
        } else {
            &[]
        }
    }
}

#[test]
fn recovery_reserves_before_callbacks_and_prepares_all_before_commit() {
    let dir = temp_dir("provider-attachment-order");
    append_wal(&dir, 0, &[node_created(1)]);
    let events = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(LifecycleProvider::new(*b"LC01", Arc::clone(&events)));
    let second = Arc::new(LifecycleProvider::new(*b"LC02", Arc::clone(&events)));

    let recovered = SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![
            first.clone() as Arc<dyn IndexProvider>,
            second.clone() as Arc<dyn IndexProvider>,
        ],
    )
    .unwrap();
    let events = events.lock();
    let first_callback = events
        .iter()
        .position(|event| event.ends_with(":change"))
        .unwrap();
    let last_reserve = events
        .iter()
        .rposition(|event| event.ends_with(":reserve"))
        .unwrap();
    let first_commit = events
        .iter()
        .position(|event| event.ends_with(":commit"))
        .unwrap();
    let last_prepare = events
        .iter()
        .rposition(|event| event.ends_with(":prepare"))
        .unwrap();

    assert!(last_reserve < first_callback);
    assert!(last_prepare < first_commit);
    assert!(first.state.lock().attached);
    assert!(second.state.lock().attached);
    drop(events);
    drop(recovered);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recovery_callback_and_prepare_failures_abort_every_reservation() {
    let callback_dir = temp_dir("provider-attachment-callback-abort");
    append_wal(&callback_dir, 0, &[node_created(1)]);
    let callback_events = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(LifecycleProvider::new(
        *b"LF01",
        Arc::clone(&callback_events),
    ));
    let mut failing = LifecycleProvider::new(*b"LF02", Arc::clone(&callback_events));
    failing.fail_change = true;
    let failing = Arc::new(failing);
    assert!(
        SharedGraph::recover_with_providers(
            &callback_dir,
            GraphId::new(7),
            vec![
                first.clone() as Arc<dyn IndexProvider>,
                failing.clone() as Arc<dyn IndexProvider>,
            ],
        )
        .is_err()
    );
    assert_eq!(first.state.lock().aborts, 1);
    assert_eq!(failing.state.lock().aborts, 1);
    assert!(!first.state.lock().attached);
    assert!(!failing.state.lock().attached);
    assert!(
        callback_events
            .lock()
            .iter()
            .all(|event| !event.ends_with(":commit"))
    );

    let prepare_dir = temp_dir("provider-attachment-prepare-abort");
    let prepare_events = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(LifecycleProvider::new(
        *b"LP01",
        Arc::clone(&prepare_events),
    ));
    let mut failing = LifecycleProvider::new(*b"LP02", Arc::clone(&prepare_events));
    failing.fail_prepare = true;
    let failing = Arc::new(failing);
    assert!(
        SharedGraph::recover_with_providers(
            &prepare_dir,
            GraphId::new(7),
            vec![
                first.clone() as Arc<dyn IndexProvider>,
                failing.clone() as Arc<dyn IndexProvider>,
            ],
        )
        .is_err()
    );
    assert_eq!(first.state.lock().aborts, 1);
    assert_eq!(failing.state.lock().aborts, 1);
    assert!(!first.state.lock().attached);
    assert!(
        prepare_events
            .lock()
            .iter()
            .all(|event| !event.ends_with(":commit"))
    );

    let _ = std::fs::remove_dir_all(callback_dir);
    let _ = std::fs::remove_dir_all(prepare_dir);
}

#[test]
fn partial_reservation_failure_aborts_only_prior_owned_providers() {
    let dir = temp_dir("provider-attachment-reserve-abort");
    let events = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(LifecycleProvider::new(*b"LR01", Arc::clone(&events)));
    let mut failing = LifecycleProvider::new(*b"LR02", events);
    failing.fail_reserve = true;
    let failing = Arc::new(failing);
    failing.state.lock().reserved = true;

    assert!(
        SharedGraph::recover_with_providers(
            &dir,
            GraphId::new(7),
            vec![
                first.clone() as Arc<dyn IndexProvider>,
                failing.clone() as Arc<dyn IndexProvider>,
            ],
        )
        .is_err()
    );
    assert_eq!(first.state.lock().aborts, 1);
    assert_eq!(failing.state.lock().aborts, 0);
    assert!(!first.state.lock().attached);
    assert!(!failing.state.lock().attached);
    assert!(failing.state.lock().reserved);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recovery_rebuild_and_construction_failures_abort_owned_reservations() {
    let rebuild_dir = temp_dir("provider-attachment-rebuild-abort");
    write_snapshot(&rebuild_dir, &SharedGraph::new(GraphId::new(7)), 1);
    let rebuild_events = Arc::new(Mutex::new(Vec::new()));
    let mut rebuilding = LifecycleProvider::new(*b"LB01", rebuild_events);
    rebuilding.declares_state = true;
    rebuilding.fail_rebuild = true;
    let rebuilding = Arc::new(rebuilding);

    assert!(
        SharedGraph::recover_with_providers(
            &rebuild_dir,
            GraphId::new(7),
            vec![rebuilding.clone() as Arc<dyn IndexProvider>],
        )
        .is_err()
    );
    assert_eq!(rebuilding.state.lock().aborts, 1);
    assert!(!rebuilding.state.lock().attached);

    let construction_dir = temp_dir("provider-attachment-construction-abort");
    let construction_events = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(LifecycleProvider::new(*b"LB02", construction_events));
    set_before_shared_construction_hook(|| {
        Err(GraphError::Inconsistent {
            reason: "synthetic final SharedGraph construction failure".to_owned(),
        })
    });
    assert!(
        SharedGraph::recover_with_providers(
            &construction_dir,
            GraphId::new(7),
            vec![provider.clone() as Arc<dyn IndexProvider>],
        )
        .is_err()
    );
    assert_eq!(provider.state.lock().aborts, 1);
    assert!(!provider.state.lock().attached);

    let _ = std::fs::remove_dir_all(rebuild_dir);
    let _ = std::fs::remove_dir_all(construction_dir);
}
