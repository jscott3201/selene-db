//! Reserved label prefixes for engine-internal graph metadata.
//!
//! Canonical home: this prefix was consolidated into `selene-core` during the
//! procedure-pack teardown. The native built-in procedures and any future
//! reserved-label policy read this constant from `selene-core` so the prefix has
//! a single source of truth.

/// Node label prefix reserved for selene-internal graph metadata.
///
/// Labels beginning with this prefix are reserved for engine-owned bookkeeping
/// and must not be assigned by user mutations. Defined here (rather than in a
/// downstream crate) so the reservation is visible to every consumer.
pub const RESERVED_LABEL_PREFIX: &str = "selene_";
