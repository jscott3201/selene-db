//! Lifecycle script runner for pack snapshots.

use std::sync::Mutex;

use jiff::Timestamp;
use selene_core::intern;
use selene_pack::{
    ActivationError, ActivationRegistry, Active, Deprecated, LifecycleEvent, LifecycleSink,
    Principal, Staged, Uploaded, Validating,
};
use selene_testing::{PackLifecycleSinkMode, PackLifecycleStep};

use super::manifest_materializer::upload_bytes;

pub struct LifecycleRunResult {
    pub events: Vec<LifecycleEvent>,
    pub registry: ActivationRegistry,
    pub error: Option<ActivationError>,
}

pub fn run_lifecycle_script(
    script: &[PackLifecycleStep],
    sink_mode: PackLifecycleSinkMode,
) -> LifecycleRunResult {
    let sink = TestSink::new(sink_mode);
    let mut registry = ActivationRegistry::new();
    let mut state = LifecycleState::Empty;
    let mut error = None;

    for (step_index, step) in script.iter().enumerate() {
        let now = timestamp(step_index as i64);
        match step {
            PackLifecycleStep::Upload {
                raw_bytes,
                principal,
            } => {
                state = LifecycleState::Uploaded(Uploaded::new(
                    upload_bytes(raw_bytes),
                    principal_from_str(principal),
                    now,
                ));
            }
            PackLifecycleStep::Validate => {
                let LifecycleState::Uploaded(uploaded) = state else {
                    panic!("Validate step requires Uploaded state");
                };
                match uploaded.validate(&sink, now) {
                    Ok(validating) => state = LifecycleState::Validating(validating),
                    Err(err) => {
                        state = LifecycleState::Failed;
                        error = Some(err);
                    }
                }
            }
            PackLifecycleStep::Stage => {
                let LifecycleState::Validating(validating) = state else {
                    panic!("Stage step requires Validating state");
                };
                match validating.stage(&sink, now) {
                    Ok(staged) => state = LifecycleState::Staged(staged),
                    Err(err) => {
                        error = Some(err);
                        break;
                    }
                }
            }
            PackLifecycleStep::Activate => {
                let LifecycleState::Staged(staged) = state else {
                    panic!("Activate step requires Staged state");
                };
                match staged.activate(&sink, &mut registry, now) {
                    Ok(active) => state = LifecycleState::Active(active),
                    Err(err) => {
                        error = Some(err);
                        break;
                    }
                }
            }
            PackLifecycleStep::Deprecate { reason } => {
                let LifecycleState::Active(active) = state else {
                    panic!("Deprecate step requires Active state");
                };
                match active.deprecate(&sink, &mut registry, intern_str(reason), now) {
                    Ok(deprecated) => state = LifecycleState::Deprecated(deprecated),
                    Err(err) => {
                        error = Some(err);
                        break;
                    }
                }
            }
            PackLifecycleStep::Disable => {
                let LifecycleState::Deprecated(deprecated) = state else {
                    panic!("Disable step requires Deprecated state");
                };
                match deprecated.disable(&sink, &mut registry, now) {
                    Ok(_disabled) => state = LifecycleState::Disabled,
                    Err(err) => {
                        error = Some(err);
                        break;
                    }
                }
            }
            PackLifecycleStep::AssertValidationFailed => {
                assert!(
                    matches!(error, Some(ActivationError::Manifest { .. })),
                    "AssertValidationFailed requires prior manifest validation error"
                );
            }
            _ => unreachable!("unknown lifecycle step"),
        }
    }

    LifecycleRunResult {
        events: sink.events(),
        registry,
        error,
    }
}

enum LifecycleState {
    Empty,
    Uploaded(Uploaded),
    Validating(Validating),
    Staged(Staged),
    Active(Active),
    Deprecated(Deprecated),
    Disabled,
    Failed,
}

struct TestSink {
    mode: PackLifecycleSinkMode,
    events: Mutex<Vec<LifecycleEvent>>,
    call_count: Mutex<usize>,
}

impl TestSink {
    fn new(mode: PackLifecycleSinkMode) -> Self {
        Self {
            mode,
            events: Mutex::new(Vec::new()),
            call_count: Mutex::new(0),
        }
    }

    fn events(&self) -> Vec<LifecycleEvent> {
        self.events.lock().expect("events lock").clone()
    }
}

impl LifecycleSink for TestSink {
    fn record(&self, event: &LifecycleEvent) -> Result<(), ActivationError> {
        let mut call_count = self.call_count.lock().expect("call-count lock");
        *call_count += 1;
        if let PackLifecycleSinkMode::RefuseAtStep { step_index, detail } = self.mode
            && *call_count == step_index
        {
            return Err(ActivationError::SinkRefused {
                detail: detail.to_owned(),
            });
        }
        self.events.lock().expect("events lock").push(event.clone());
        Ok(())
    }
}

fn principal_from_str(value: &str) -> Principal {
    Principal::new(intern_str(value))
}

fn intern_str(value: &str) -> selene_core::IStr {
    intern(value).expect("test string interns")
}

fn timestamp(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("test timestamp in range")
}
