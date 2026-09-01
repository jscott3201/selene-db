//! Recovery attachment reservation and publication lifecycle tests.

use std::sync::Arc;

use parking_lot::Mutex;
use selene_core::{Change, GraphId};

use super::super::set_before_shared_construction_hook;
use super::{append_wal, node_created, temp_dir, write_snapshot};
use crate::{GraphError, IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};

const LIFECYCLE_SUB_TAGS: &[SubTag] = &[SubTag(*b"LIFE")];

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
