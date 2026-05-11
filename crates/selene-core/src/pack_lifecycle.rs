//! Procedure-pack lifecycle audit payloads.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::IStr;

/// Procedure-pack lifecycle audit event.
///
/// This mirrors `selene-pack`'s public `LifecycleEvent` shape while using
/// interned strings for WAL/change payload consistency in `selene-core`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[non_exhaustive]
pub enum PackLifecycleEvent {
    /// Manifest validation failed before staging.
    ValidationFailed {
        /// Parsed pack name when available.
        pack_name: Option<IStr>,
        /// Caller identity that attempted activation.
        principal: IStr,
        /// Human-readable validation error.
        ///
        /// Free-form diagnostic text produced by manifest parsing. Stored as
        /// `String` rather than interned `IStr` because the contents vary
        /// with every malformed input (offsets, snippets, etc.); admitting
        /// each variant into the global interner would eventually exhaust
        /// the cap and start failing unrelated lifecycle records with
        /// `SinkRefused`.
        error: String,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
    /// Pack reached the staged state.
    Staged {
        /// Pack name.
        pack_name: IStr,
        /// Pack content hash bytes.
        content_hash: [u8; 32],
        /// Caller identity that staged the pack.
        principal: IStr,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
    /// Pack became active.
    Activated {
        /// Pack name.
        pack_name: IStr,
        /// Pack content hash bytes.
        content_hash: [u8; 32],
        /// Caller identity that activated the pack.
        principal: IStr,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
    /// Pack was deprecated but still occupies its name.
    Deprecated {
        /// Pack name.
        pack_name: IStr,
        /// Deprecation reason.
        reason: IStr,
        /// Caller identity that deprecated the pack.
        principal: IStr,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
    /// Pack was disabled and removed from the occupancy registry.
    Disabled {
        /// Pack name.
        pack_name: IStr,
        /// Caller identity that disabled the pack.
        principal: IStr,
        /// Caller-injected event timestamp.
        at: Timestamp,
    },
}
