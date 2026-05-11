//! Lifecycle event sink abstraction.

use jiff::Timestamp;
use selene_core::IStr;

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
