//! Procedure-pack activation lifecycle tests.

use std::sync::Mutex;

use jiff::Timestamp;
use selene_core::intern;
use selene_pack::{
    ACTIVATION_SEAL_COVERAGE, ActivationError, ActivationRegistry, ActivationStatus, ContentHash,
    DEFERRED_GATES, Gate, LifecycleEvent, LifecycleSink, NoopSink, PLACEHOLDER_CONTENT_HASH,
    Principal, Uploaded,
};
use serde_json::{Value, json};

#[derive(Default)]
struct TestSink {
    events: Mutex<Vec<LifecycleEvent>>,
}

impl TestSink {
    fn events(&self) -> Vec<LifecycleEvent> {
        self.events.lock().expect("sink lock not poisoned").clone()
    }
}

impl LifecycleSink for TestSink {
    fn record(&self, event: &LifecycleEvent) -> Result<(), ActivationError> {
        self.events
            .lock()
            .expect("sink lock not poisoned")
            .push(event.clone());
        Ok(())
    }
}

struct RefusingSink;

impl LifecycleSink for RefusingSink {
    fn record(&self, _event: &LifecycleEvent) -> Result<(), ActivationError> {
        Err(ActivationError::SinkRefused {
            detail: "test sink refused event".to_owned(),
        })
    }
}

fn ts(seconds: i64) -> Timestamp {
    Timestamp::new(seconds, 0).expect("test timestamp in range")
}

fn principal() -> Principal {
    Principal::new(intern("activation.test.principal").expect("test principal interns"))
}

fn reason() -> selene_core::IStr {
    intern("activation.test.reason").expect("test reason interns")
}

fn manifest_bytes(pack_name: &str) -> Vec<u8> {
    serde_json::to_vec(&manifest_value(pack_name)).expect("manifest JSON serializes")
}

fn manifest_value(pack_name: &str) -> Value {
    json!({
        "schema_version": 1,
        "pack_name": pack_name,
        "pack_version": "1.2.3",
        "content_hash": PLACEHOLDER_CONTENT_HASH,
        "procedures": [
            {
                "name": format!("{pack_name}.echo"),
                "tier": "graph",
                "mutability": "read",
                "input_schema": { "inline": { "type": "object" } },
                "output_schema": { "inline": { "type": "object" } },
                "capability_required": null
            }
        ]
    })
}

fn uploaded(pack_name: &str) -> Uploaded {
    Uploaded::new(manifest_bytes(pack_name), principal(), ts(1))
}

fn staged(pack_name: &str, sink: &dyn LifecycleSink) -> selene_pack::Staged {
    uploaded(pack_name)
        .validate(sink, ts(2))
        .expect("manifest validates")
        .stage(sink, ts(3))
        .expect("pack stages")
}

fn active(
    pack_name: &str,
    sink: &dyn LifecycleSink,
    registry: &mut ActivationRegistry,
) -> selene_pack::Active {
    staged(pack_name, sink)
        .activate(sink, registry, ts(4))
        .expect("pack activates")
}

#[test]
fn linear_lifecycle_succeeds() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let active = active("demo_pack", &sink, &mut registry);
    let deprecated = active
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates");
    let disabled = deprecated
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");

    assert_eq!(disabled.pack_name(), "demo_pack");
    assert!(!registry.is_occupied("demo_pack"));
}

#[test]
fn staged_event_emitted_with_content_hash_placeholder() {
    let sink = TestSink::default();
    let staged = staged("demo_pack", &sink);

    assert_eq!(staged.content_hash(), ContentHash::placeholder());
    assert!(matches!(
        &sink.events()[0],
        LifecycleEvent::Staged {
            pack_name,
            content_hash,
            ..
        } if pack_name == "demo_pack" && content_hash.is_placeholder()
    ));
}

#[test]
fn activated_event_emitted_after_activate() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let _active = active("demo_pack", &sink, &mut registry);

    assert!(matches!(
        &sink.events()[1],
        LifecycleEvent::Activated { pack_name, .. } if pack_name == "demo_pack"
    ));
}

#[test]
fn deprecated_event_emitted_with_reason() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let active = active("demo_pack", &sink, &mut registry);
    let reason = reason();
    let _deprecated = active
        .deprecate(&sink, &mut registry, reason, ts(5))
        .expect("pack deprecates");

    assert!(matches!(
        &sink.events()[2],
        LifecycleEvent::Deprecated {
            pack_name,
            reason: event_reason,
            ..
        } if pack_name == "demo_pack" && *event_reason == reason
    ));
}

#[test]
fn disabled_event_emitted_at_terminal() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let active = active("demo_pack", &sink, &mut registry);
    let disabled = active
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates")
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");

    assert_eq!(disabled.disabled_at(), ts(6));
    assert!(matches!(
        &sink.events()[3],
        LifecycleEvent::Disabled { pack_name, .. } if pack_name == "demo_pack"
    ));
}

#[test]
fn invalid_manifest_routes_to_validation_failed_event() {
    let sink = TestSink::default();
    let err = Uploaded::new(b"{".to_vec(), principal(), ts(1))
        .validate(&sink, ts(2))
        .expect_err("invalid JSON fails validation");

    assert!(matches!(err, ActivationError::Manifest { .. }));
    assert!(matches!(
        &sink.events()[0],
        LifecycleEvent::ValidationFailed {
            pack_name: None,
            ..
        }
    ));
}

#[test]
fn validation_failed_event_carries_optional_pack_name() {
    let sink = TestSink::default();
    let mut value = manifest_value("demo_pack");
    value["content_hash"] = json!("sha256:bad");
    let bytes = serde_json::to_vec(&value).expect("manifest JSON serializes");

    let _err = Uploaded::new(bytes, principal(), ts(1))
        .validate(&sink, ts(2))
        .expect_err("invalid content hash fails validation");

    assert!(matches!(
        &sink.events()[0],
        LifecycleEvent::ValidationFailed {
            pack_name: Some(pack_name),
            ..
        } if pack_name == "demo_pack"
    ));
}

#[test]
fn staged_to_active_rejects_duplicate_pack_name() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let _active = active("demo_pack", &sink, &mut registry);
    let err = staged("demo_pack", &sink)
        .activate(&sink, &mut registry, ts(7))
        .expect_err("duplicate pack activation fails");

    assert!(matches!(
        err,
        ActivationError::PackAlreadyActive { ref pack_name } if pack_name == "demo_pack"
    ));
}

#[test]
fn deprecated_pack_still_occupies_name() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let active = active("demo_pack", &sink, &mut registry);
    let _deprecated = active
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates");

    assert!(registry.is_deprecated("demo_pack"));
    assert!(matches!(
        staged("demo_pack", &sink).activate(&sink, &mut registry, ts(7)),
        Err(ActivationError::PackAlreadyActive { .. })
    ));
}

#[test]
fn disabled_pack_frees_name() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let active_state = active("demo_pack", &sink, &mut registry);
    let _disabled = active_state
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates")
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");

    let reactivated = active("demo_pack", &sink, &mut registry);

    assert_eq!(reactivated.pack_name(), "demo_pack");
    assert!(registry.is_active("demo_pack"));
}

#[test]
fn independent_pack_names_activate_concurrently() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let _a = active("pack_a", &sink, &mut registry);
    let _b = active("pack_b", &sink, &mut registry);

    assert!(registry.is_active("pack_a"));
    assert!(registry.is_active("pack_b"));
    assert_eq!(registry.len(), 2);
}

#[test]
fn activate_inserts_active_entry() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let _active = active("demo_pack", &sink, &mut registry);
    let entry = registry.get("demo_pack").expect("entry inserted");

    assert_eq!(entry.pack_name(), "demo_pack");
    assert_eq!(entry.status(), ActivationStatus::Active);
}

#[test]
fn disable_removes_entry() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let active = active("demo_pack", &sink, &mut registry);
    let _disabled = active
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates")
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");

    assert!(!registry.is_occupied("demo_pack"));
    assert!(registry.is_empty());
}

#[test]
fn failing_sink_leaves_registry_unchanged() {
    let sink = TestSink::default();
    let refusing = RefusingSink;
    let mut registry = ActivationRegistry::new();
    let staged = staged("demo_pack", &sink);
    let err = staged
        .activate(&refusing, &mut registry, ts(4))
        .expect_err("refusing sink aborts activation");

    assert!(matches!(err, ActivationError::SinkRefused { .. }));
    assert!(!registry.is_occupied("demo_pack"));
}

#[test]
fn disabled_has_no_transitions() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let disabled = active("demo_pack", &sink, &mut registry)
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates")
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");

    // Would not compile: `Disabled` intentionally exposes no transition methods.
    // let _ = disabled.activate(&sink, &mut registry, ts(7));
    assert_eq!(disabled.pack_name(), "demo_pack");
}

#[test]
fn activation_error_manifest_delegates_to_source_gate() {
    let sink = TestSink::default();
    let err = Uploaded::new(b"{".to_vec(), principal(), ts(1))
        .validate(&sink, ts(2))
        .expect_err("invalid JSON fails validation");

    assert_eq!(err.gate(), Some(Gate::ManifestSyntaxAndSchema));
}

#[test]
fn activation_error_pack_already_active_maps_to_registry_conflict_detection() {
    let err = ActivationError::PackAlreadyActive {
        pack_name: "demo_pack".to_owned(),
    };

    assert_eq!(err.gate(), Some(Gate::RegistryConflictDetection));
}

#[test]
fn activation_error_sink_refused_has_no_gate() {
    let err = ActivationError::SinkRefused {
        detail: "test".to_owned(),
    };

    assert_eq!(err.gate(), None);
}

#[test]
fn principal_round_trips_through_all_events() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let principal = principal();
    let active = Uploaded::new(manifest_bytes("demo_pack"), principal, ts(1))
        .validate(&sink, ts(2))
        .expect("manifest validates")
        .stage(&sink, ts(3))
        .expect("pack stages")
        .activate(&sink, &mut registry, ts(4))
        .expect("pack activates");
    let _disabled = active
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates")
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");

    for event in sink.events() {
        let event_principal = match event {
            LifecycleEvent::ValidationFailed { principal, .. }
            | LifecycleEvent::Staged { principal, .. }
            | LifecycleEvent::Activated { principal, .. }
            | LifecycleEvent::Deprecated { principal, .. }
            | LifecycleEvent::Disabled { principal, .. } => principal,
            _ => unreachable!("BRIEF-45 lifecycle event shape is covered"),
        };
        assert_eq!(event_principal, principal);
    }
}

#[test]
fn timestamp_injected_argument_is_recorded_verbatim() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let _active = staged("demo_pack", &sink)
        .activate(&sink, &mut registry, ts(44))
        .expect("pack activates");

    assert!(matches!(
        &sink.events()[1],
        LifecycleEvent::Activated { at, .. } if *at == ts(44)
    ));
}

#[test]
fn content_hash_placeholder_at_staged_and_active() {
    let sink = TestSink::default();
    let mut registry = ActivationRegistry::new();
    let staged = staged("demo_pack", &sink);

    assert!(staged.content_hash().is_placeholder());
    let active = staged
        .activate(&sink, &mut registry, ts(4))
        .expect("pack activates");
    assert!(active.content_hash().is_placeholder());
}

#[test]
fn gate_partition_drift_test_still_passes() {
    for gate in Gate::ALL {
        let count = usize::from(selene_pack::MANIFEST_VALIDATION_COVERAGE.contains(gate))
            + usize::from(ACTIVATION_SEAL_COVERAGE.contains(gate))
            + usize::from(DEFERRED_GATES.contains(gate));

        assert_eq!(count, 1, "gate {} appears {count} times", gate.id());
    }
    assert!(ACTIVATION_SEAL_COVERAGE.contains(&Gate::RegistryConflictDetection));
    assert!(!DEFERRED_GATES.contains(&Gate::RegistryConflictDetection));
}

#[test]
fn noop_sink_does_not_panic() {
    let sink = NoopSink;
    let mut registry = ActivationRegistry::new();
    let _disabled = active("demo_pack", &sink, &mut registry)
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates")
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");
}
