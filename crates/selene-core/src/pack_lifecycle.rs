//! Procedure-pack lifecycle audit payloads.

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{CoreResult, IStr, Value, intern};

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

impl PackLifecycleEvent {
    /// Factory table with one sample event for each [`PackLifecycleEvent`] variant.
    ///
    /// Tests use this as an append-only ANCHOR so new lifecycle variants are
    /// mirrored in `selene-core` before downstream consumers can drift.
    pub const ALL: &[fn() -> Self] = &[
        || Self::ValidationFailed {
            pack_name: Some(pack_lifecycle_variant_istr("pack.lifecycle.all.pack")),
            principal: pack_lifecycle_variant_istr("pack.lifecycle.all.principal"),
            error: "pack.lifecycle.all.error".to_owned(),
            at: pack_lifecycle_timestamp(1),
        },
        || Self::Staged {
            pack_name: pack_lifecycle_variant_istr("pack.lifecycle.all.pack"),
            content_hash: [1_u8; 32],
            principal: pack_lifecycle_variant_istr("pack.lifecycle.all.principal"),
            at: pack_lifecycle_timestamp(2),
        },
        || Self::Activated {
            pack_name: pack_lifecycle_variant_istr("pack.lifecycle.all.pack"),
            content_hash: [2_u8; 32],
            principal: pack_lifecycle_variant_istr("pack.lifecycle.all.principal"),
            at: pack_lifecycle_timestamp(3),
        },
        || Self::Deprecated {
            pack_name: pack_lifecycle_variant_istr("pack.lifecycle.all.pack"),
            reason: pack_lifecycle_variant_istr("pack.lifecycle.all.reason"),
            principal: pack_lifecycle_variant_istr("pack.lifecycle.all.principal"),
            at: pack_lifecycle_timestamp(4),
        },
        || Self::Disabled {
            pack_name: pack_lifecycle_variant_istr("pack.lifecycle.all.pack"),
            principal: pack_lifecycle_variant_istr("pack.lifecycle.all.principal"),
            at: pack_lifecycle_timestamp(5),
        },
    ];

    /// Number of known [`PackLifecycleEvent`] variants in this build.
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

    /// Convert this event into the canonical `selene.pack.history` row payload.
    ///
    /// The row shape is the primitive core representation consumed by
    /// `selene-pack`: `[kind, pack_name, content_hash, principal, reason,
    /// error, at]`, with absent nullable columns represented as [`Value::Null`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::CoreError::IStrCapExceeded`] if the row kind string
    /// cannot be interned.
    pub fn to_history_row(&self) -> CoreResult<Vec<Value>> {
        self.to_history_row_with(intern)
    }

    /// Convert this event into the canonical `selene.pack.history` row payload
    /// using the caller's interning function for row-kind strings.
    ///
    /// This keeps the exhaustive mapping in `selene-core` while letting
    /// downstream crates map interner failures into their local error type.
    ///
    /// # Errors
    ///
    /// Returns the error produced by `intern_value`.
    pub fn to_history_row_with<E>(
        &self,
        mut intern_value: impl FnMut(&str) -> Result<IStr, E>,
    ) -> Result<Vec<Value>, E> {
        match self {
            Self::ValidationFailed {
                pack_name,
                principal,
                error,
                at,
            } => Ok(vec![
                pack_lifecycle_string_value("validation_failed", &mut intern_value)?,
                pack_lifecycle_optional_string_value(*pack_name),
                Value::Null,
                Value::String(*principal),
                Value::Null,
                pack_lifecycle_external_string_value(error),
                pack_lifecycle_timestamp_value(*at),
            ]),
            Self::Staged {
                pack_name,
                content_hash,
                principal,
                at,
            } => Ok(vec![
                pack_lifecycle_string_value("staged", &mut intern_value)?,
                Value::String(*pack_name),
                pack_lifecycle_external_string_value(&pack_lifecycle_content_hash_string(
                    content_hash,
                )),
                Value::String(*principal),
                Value::Null,
                Value::Null,
                pack_lifecycle_timestamp_value(*at),
            ]),
            Self::Activated {
                pack_name,
                content_hash,
                principal,
                at,
            } => Ok(vec![
                pack_lifecycle_string_value("activated", &mut intern_value)?,
                Value::String(*pack_name),
                pack_lifecycle_external_string_value(&pack_lifecycle_content_hash_string(
                    content_hash,
                )),
                Value::String(*principal),
                Value::Null,
                Value::Null,
                pack_lifecycle_timestamp_value(*at),
            ]),
            Self::Deprecated {
                pack_name,
                reason,
                principal,
                at,
            } => Ok(vec![
                pack_lifecycle_string_value("deprecated", &mut intern_value)?,
                Value::String(*pack_name),
                Value::Null,
                Value::String(*principal),
                Value::String(*reason),
                Value::Null,
                pack_lifecycle_timestamp_value(*at),
            ]),
            Self::Disabled {
                pack_name,
                principal,
                at,
            } => Ok(vec![
                pack_lifecycle_string_value("disabled", &mut intern_value)?,
                Value::String(*pack_name),
                Value::Null,
                Value::String(*principal),
                Value::Null,
                Value::Null,
                pack_lifecycle_timestamp_value(*at),
            ]),
        }
    }
}

fn pack_lifecycle_variant_istr(name: &str) -> IStr {
    intern(name).expect("PackLifecycleEvent::ALL fixture strings fit the process interner cap")
}

fn pack_lifecycle_timestamp(second: i64) -> Timestamp {
    Timestamp::new(second, 0).expect("PackLifecycleEvent::ALL timestamp fixture is in range")
}

fn pack_lifecycle_optional_string_value(value: Option<IStr>) -> Value {
    value.map_or(Value::Null, Value::String)
}

fn pack_lifecycle_string_value<E>(
    value: &str,
    intern_value: &mut impl FnMut(&str) -> Result<IStr, E>,
) -> Result<Value, E> {
    intern_value(value).map(Value::String)
}

fn pack_lifecycle_external_string_value(value: &str) -> Value {
    Value::ExternalString(Arc::from(value))
}

fn pack_lifecycle_timestamp_value(at: Timestamp) -> Value {
    Value::ZonedDateTime(at.to_zoned(jiff::tz::TimeZone::UTC))
}

fn pack_lifecycle_content_hash_string(hash: &[u8; 32]) -> String {
    let mut out = String::with_capacity("blake3:".len() + hash.len() * 2);
    out.push_str("blake3:");
    out.push_str(&pack_lifecycle_hex_lower(hash));
    out
}

fn pack_lifecycle_hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_lifecycle_event_all_covers_every_variant() {
        assert_eq!(PackLifecycleEvent::VARIANT_COUNT, 5);
        let mut discriminants = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for factory in PackLifecycleEvent::ALL {
            let event = factory();
            assert!(
                discriminants.insert(std::mem::discriminant(&event)),
                "PackLifecycleEvent::ALL has duplicate variant: {}",
                event.variant_name()
            );
            let name = event.variant_name();
            assert!(
                !name.is_empty(),
                "PackLifecycleEvent::variant_name must not be empty"
            );
            assert!(
                names.insert(name),
                "PackLifecycleEvent::variant_name collision: {name}"
            );
        }
        assert_eq!(discriminants.len(), PackLifecycleEvent::ALL.len());
        assert_eq!(names.len(), PackLifecycleEvent::ALL.len());
    }

    #[test]
    fn to_history_row_uses_existing_column_shape() {
        let event = PackLifecycleEvent::Staged {
            pack_name: pack_lifecycle_variant_istr("pack.lifecycle.row.pack"),
            content_hash: [0xab; 32],
            principal: pack_lifecycle_variant_istr("pack.lifecycle.row.principal"),
            at: pack_lifecycle_timestamp(1),
        };

        let row = event.to_history_row().expect("row conversion succeeds");
        assert_eq!(row.len(), 7);
        assert!(matches!(row[0], Value::String(kind) if kind.as_str() == "staged"));
        assert!(matches!(&row[2], Value::ExternalString(hash) if hash.starts_with("blake3:abab")));
    }
}
