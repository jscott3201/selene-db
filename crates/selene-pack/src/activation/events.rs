//! Lifecycle event sink abstraction.

use jiff::Timestamp;
use selene_core::{IStr, intern};

use super::{ActivationError, ContentHash, Principal};

/// Auditable procedure-pack lifecycle event.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LifecycleEvent {
    /// Manifest validation failed before staging.
    ValidationFailed {
        /// Parsed pack name when available.
        pack_name: Option<String>,
        /// Caller identity that attempted activation.
        principal: Principal,
        /// Human-readable validation error.
        error: String,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
    /// Pack validated and reached the staged state.
    Staged {
        /// Pack name.
        pack_name: String,
        /// Pack content hash.
        content_hash: ContentHash,
        /// Caller identity that staged the pack.
        principal: Principal,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
    /// Pack became active.
    Activated {
        /// Pack name.
        pack_name: String,
        /// Pack content hash.
        content_hash: ContentHash,
        /// Caller identity that activated the pack.
        principal: Principal,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
    /// Pack was deprecated but still occupies its name.
    Deprecated {
        /// Pack name.
        pack_name: String,
        /// Deprecation reason.
        reason: IStr,
        /// Caller identity that deprecated the pack.
        principal: Principal,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
    /// Pack was disabled and removed from the occupancy registry.
    Disabled {
        /// Pack name.
        pack_name: String,
        /// Caller identity that disabled the pack.
        principal: Principal,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
}

impl LifecycleEvent {
    /// Factory table with one sample event for each [`LifecycleEvent`] variant.
    ///
    /// This mirrors [`selene_core::PackLifecycleEvent::ALL`] by variant name;
    /// field representations differ across the activation and WAL layers.
    pub const ALL: &[fn() -> Self] = &[
        || Self::ValidationFailed {
            pack_name: Some("pack.lifecycle.all.pack".to_owned()),
            principal: lifecycle_principal(),
            error: "pack.lifecycle.all.error".to_owned(),
            at: lifecycle_timestamp(1),
        },
        || Self::Staged {
            pack_name: "pack.lifecycle.all.pack".to_owned(),
            content_hash: ContentHash::compute(b"pack.lifecycle.all.staged"),
            principal: lifecycle_principal(),
            at: lifecycle_timestamp(2),
        },
        || Self::Activated {
            pack_name: "pack.lifecycle.all.pack".to_owned(),
            content_hash: ContentHash::compute(b"pack.lifecycle.all.activated"),
            principal: lifecycle_principal(),
            at: lifecycle_timestamp(3),
        },
        || Self::Deprecated {
            pack_name: "pack.lifecycle.all.pack".to_owned(),
            reason: lifecycle_istr("pack.lifecycle.all.reason"),
            principal: lifecycle_principal(),
            at: lifecycle_timestamp(4),
        },
        || Self::Disabled {
            pack_name: "pack.lifecycle.all.pack".to_owned(),
            principal: lifecycle_principal(),
            at: lifecycle_timestamp(5),
        },
    ];

    /// Number of known [`LifecycleEvent`] variants in this build.
    pub const VARIANT_COUNT: usize = Self::ALL.len();

    /// Stable telemetry name for this lifecycle event variant.
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::ValidationFailed { .. } => "ValidationFailed",
            Self::Staged { .. } => "Staged",
            Self::Activated { .. } => "Activated",
            Self::Deprecated { .. } => "Deprecated",
            Self::Disabled { .. } => "Disabled",
        }
    }
}

fn lifecycle_istr(name: &str) -> IStr {
    intern(name).expect("LifecycleEvent::ALL fixture strings fit the process interner cap")
}

fn lifecycle_principal() -> Principal {
    Principal::new(lifecycle_istr("pack.lifecycle.all.principal"))
}

fn lifecycle_timestamp(second: i64) -> Timestamp {
    Timestamp::new(second, 0).expect("LifecycleEvent::ALL timestamp fixture is in range")
}

/// Sink for lifecycle audit events.
pub trait LifecycleSink {
    /// Record one lifecycle event.
    ///
    /// # Errors
    ///
    /// Returns [`ActivationError::SinkRefused`] or another activation error when
    /// the sink cannot durably accept the event.
    fn record(&self, event: &LifecycleEvent) -> Result<(), ActivationError>;
}

/// Lifecycle sink that accepts and drops every event.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopSink;

impl LifecycleSink for NoopSink {
    fn record(&self, _event: &LifecycleEvent) -> Result<(), ActivationError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use selene_core::PackLifecycleEvent;

    use super::*;

    #[test]
    fn lifecycle_event_all_covers_every_variant() {
        assert_eq!(LifecycleEvent::VARIANT_COUNT, 5);
        let mut discriminants = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for factory in LifecycleEvent::ALL {
            let event = factory();
            assert!(
                discriminants.insert(std::mem::discriminant(&event)),
                "LifecycleEvent::ALL has duplicate variant: {}",
                event.variant_name()
            );
            let name = event.variant_name();
            assert!(
                !name.is_empty(),
                "LifecycleEvent::variant_name must not be empty"
            );
            assert!(
                names.insert(name),
                "LifecycleEvent::variant_name collision: {name}"
            );
        }
        assert_eq!(discriminants.len(), LifecycleEvent::ALL.len());
        assert_eq!(names.len(), LifecycleEvent::ALL.len());
    }

    #[test]
    fn lifecycle_event_mirror_shape_parity() {
        assert_eq!(
            LifecycleEvent::ALL.len(),
            PackLifecycleEvent::ALL.len(),
            "LifecycleEvent and PackLifecycleEvent must expose the same variant count"
        );
        for (left_factory, right_factory) in LifecycleEvent::ALL
            .iter()
            .zip(PackLifecycleEvent::ALL.iter())
        {
            assert_eq!(
                left_factory().variant_name(),
                right_factory().variant_name(),
                "LifecycleEvent and PackLifecycleEvent variant names must mirror"
            );
        }
    }
}
