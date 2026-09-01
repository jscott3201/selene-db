//! Recovery attachment reservation and publication lifecycle tests.

use std::sync::Arc;

use parking_lot::Mutex;
use selene_core::{Change, GraphId};

use super::super::{bounded_provider_panic, set_before_shared_construction_hook};
use super::{append_wal, node_created, temp_dir, write_snapshot};
use crate::{GraphError, IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};

const LIFECYCLE_SUB_TAGS: &[SubTag] = &[SubTag(*b"LIFE")];

#[test]
fn recovery_commit_panic_payload_text_is_bounded() {
    let payload: Box<dyn std::any::Any + Send> = Box::new("x".repeat(1_024));
    let detail = bounded_provider_panic(&payload);
    assert_eq!(detail.len(), 256);
    assert!(detail.ends_with("..."));
}

#[derive(Default)]
struct AttachmentState {
    reserved: bool,
    attached: bool,
    aborts: usize,
    live_marker: u8,
    rollback_marker: Option<u8>,
    promotions: usize,
    finalized: bool,
}

struct LifecycleProvider {
    tag: ProviderTag,
    events: Arc<Mutex<Vec<String>>>,
    state: Mutex<AttachmentState>,
    fail_reserve: bool,
    fail_change: bool,
    fail_prepare: bool,
    fail_rebuild: bool,
    fail_commit: bool,
    panic_commit: bool,
    panic_abort: bool,
    panic_finalize: bool,
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
            fail_commit: false,
            panic_commit: false,
            panic_abort: false,
            panic_finalize: false,
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

    fn commit_recovery_attachment(&self) -> Result<(), ProviderError> {
        self.event("commit");
        let mut state = self.state.lock();
        if !state.reserved || state.rollback_marker.is_some() {
            return Err(ProviderError::Inconsistent {
                reason: format!("{} commit without one prepared reservation", self.tag),
            });
        }
        state.rollback_marker = Some(state.live_marker);
        state.live_marker = 1;
        state.attached = true;
        state.promotions += 1;
        drop(state);
        assert!(!self.panic_commit, "{} commit panic failpoint", self.tag);
        if self.fail_commit {
            return Err(ProviderError::Inconsistent {
                reason: format!("{} commit failpoint", self.tag),
            });
        }
        Ok(())
    }

    fn finalize_recovery_attachment(&self) {
        self.event("finalize");
        assert!(
            !self.panic_finalize,
            "{} finalize panic failpoint",
            self.tag
        );
        let mut state = self.state.lock();
        state.rollback_marker = None;
        state.reserved = false;
        state.finalized = true;
    }

    fn abort_recovery_attachment(&self) {
        self.event("abort");
        assert!(!self.panic_abort, "{} abort panic failpoint", self.tag);
        let mut state = self.state.lock();
        if !state.reserved && state.rollback_marker.is_none() {
            return;
        }
        if let Some(prior) = state.rollback_marker.take() {
            state.live_marker = prior;
        }
        state.reserved = false;
        state.attached = false;
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
    let last_commit = events
        .iter()
        .rposition(|event| event.ends_with(":commit"))
        .unwrap();
    let first_finalize = events
        .iter()
        .position(|event| event.ends_with(":finalize"))
        .unwrap();
    let last_prepare = events
        .iter()
        .rposition(|event| event.ends_with(":prepare"))
        .unwrap();

    assert!(last_reserve < first_callback);
    assert!(last_prepare < first_commit);
    assert!(last_commit < first_finalize);
    assert!(events.iter().all(|event| !event.ends_with(":abort")));
    for provider in [&first, &second] {
        let state = provider.state.lock();
        assert!(state.attached);
        assert!(state.finalized);
        assert!(!state.reserved);
        assert_eq!(state.live_marker, 1);
        assert_eq!(state.promotions, 1);
        assert!(state.rollback_marker.is_none());
    }
    drop(events);
    drop(recovered);
    let _ = std::fs::remove_dir_all(dir);
}

fn assert_commit_failure_restores_all_live_markers(second_panics: bool) {
    let dir = temp_dir(if second_panics {
        "provider-attachment-commit-panic"
    } else {
        "provider-attachment-commit-error"
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(LifecycleProvider::new(*b"CF01", Arc::clone(&events)));
    let mut second = LifecycleProvider::new(*b"CF02", Arc::clone(&events));
    second.fail_commit = !second_panics;
    second.panic_commit = second_panics;
    let second = Arc::new(second);

    let error = match SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![
            first.clone() as Arc<dyn IndexProvider>,
            second.clone() as Arc<dyn IndexProvider>,
        ],
    ) {
        Ok(_) => panic!("commit failure must not return a SharedGraph"),
        Err(error) => error,
    };
    let GraphError::Provider(ProviderError::Inconsistent { reason }) = error else {
        panic!("expected typed provider inconsistency");
    };
    if second_panics {
        assert!(reason.contains("commit panicked at provider ordinal 2"));
        assert!(reason.contains("CF02 commit panic failpoint"));
    } else {
        assert!(reason.contains("CF02 commit failpoint"));
    }

    let events = events.lock();
    let last_prepare = events
        .iter()
        .rposition(|event| event.ends_with(":prepare"))
        .unwrap();
    let first_commit = events
        .iter()
        .position(|event| event.ends_with(":commit"))
        .unwrap();
    assert!(last_prepare < first_commit);
    assert!(events.iter().all(|event| !event.ends_with(":finalize")));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.ends_with(":abort"))
            .cloned()
            .collect::<Vec<_>>(),
        vec!["CF02:abort", "CF01:abort"]
    );
    drop(events);
    for provider in [&first, &second] {
        let state = provider.state.lock();
        assert_eq!(state.live_marker, 0);
        assert_eq!(state.promotions, 1);
        assert_eq!(state.aborts, 1);
        assert!(!state.attached);
        assert!(!state.reserved);
        assert!(state.rollback_marker.is_none());
        assert!(!state.finalized);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recovery_commit_error_reverse_aborts_and_restores_every_provider() {
    assert_commit_failure_restores_all_live_markers(false);
}

#[test]
fn recovery_commit_panic_becomes_typed_error_and_restores_every_provider() {
    assert_commit_failure_restores_all_live_markers(true);
}

#[test]
fn recovery_abort_panic_does_not_skip_earlier_provider_restoration() {
    let dir = temp_dir("provider-attachment-abort-panic");
    let events = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(LifecycleProvider::new(*b"AP01", Arc::clone(&events)));
    let mut second = LifecycleProvider::new(*b"AP02", Arc::clone(&events));
    second.fail_commit = true;
    second.panic_abort = true;
    let second = Arc::new(second);

    let error = match SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![
            first.clone() as Arc<dyn IndexProvider>,
            second.clone() as Arc<dyn IndexProvider>,
        ],
    ) {
        Ok(_) => panic!("commit error must fail recovery"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        GraphError::Provider(ProviderError::Inconsistent { reason })
            if reason.contains("AP02 commit failpoint")
    ));
    assert_eq!(
        events
            .lock()
            .iter()
            .filter(|event| event.ends_with(":abort"))
            .cloned()
            .collect::<Vec<_>>(),
        vec!["AP02:abort", "AP01:abort"]
    );
    let first_state = first.state.lock();
    assert_eq!(first_state.live_marker, 0);
    assert_eq!(first_state.aborts, 1);
    assert!(first_state.rollback_marker.is_none());
    drop(first_state);
    let second_state = second.state.lock();
    assert_eq!(second_state.live_marker, 1);
    assert_eq!(second_state.aborts, 0);
    assert_eq!(second_state.rollback_marker, Some(0));
    drop(second_state);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recovery_finalize_panic_does_not_skip_later_cleanup() {
    let dir = temp_dir("provider-attachment-finalize-panic");
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut first = LifecycleProvider::new(*b"FP01", Arc::clone(&events));
    first.panic_finalize = true;
    let first = Arc::new(first);
    let second = Arc::new(LifecycleProvider::new(*b"FP02", Arc::clone(&events)));

    let recovered = SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![
            first.clone() as Arc<dyn IndexProvider>,
            second.clone() as Arc<dyn IndexProvider>,
        ],
    )
    .expect("finalize cleanup panic must not discard recovered graph");
    let events = events.lock();
    let last_commit = events
        .iter()
        .rposition(|event| event.ends_with(":commit"))
        .unwrap();
    let first_finalize = events
        .iter()
        .position(|event| event.ends_with(":finalize"))
        .unwrap();
    assert!(last_commit < first_finalize);
    assert_eq!(events.last().unwrap(), "FP02:finalize");
    assert!(events.iter().all(|event| !event.ends_with(":abort")));
    drop(events);
    assert_eq!(first.state.lock().live_marker, 1);
    let second_state = second.state.lock();
    assert_eq!(second_state.live_marker, 1);
    assert!(second_state.finalized);
    assert!(second_state.rollback_marker.is_none());
    drop(second_state);
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
