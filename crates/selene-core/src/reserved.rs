//! Reserved label prefixes for engine-internal graph metadata.
//!
//! Canonical home (relocated from the deprecated `selene-pack` reserved module).
//! The native built-in procedures and any future reserved-label policy read this
//! constant from `selene-core` so the prefix has a single source of truth that
//! survives the procedure-pack teardown.

/// Node label prefix reserved for selene-internal graph metadata.
///
/// Labels beginning with this prefix are reserved for engine-owned bookkeeping
/// and must not be assigned by user mutations. Defined here (rather than in a
/// downstream crate) so the reservation is visible to every consumer without a
/// dependency on the procedure-pack apparatus.
pub const RESERVED_LABEL_PREFIX: &str = "selene_";
