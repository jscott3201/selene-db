//! Procedure-pack manifest accessor for the future `vector.*` procedure set.

/// Embedded BRIEF-57 procedure-pack manifest stub.
///
/// The manifest has `pack_name = "vector"` and no procedures yet. Procedure
/// registration is deferred to future `selene-vector-pack` work.
pub const STUB_MANIFEST_JSON: &str = include_str!("../resources/procedure-pack.json");

/// Return the embedded procedure-pack manifest stub.
#[must_use]
pub const fn pack_manifest() -> &'static str {
    STUB_MANIFEST_JSON
}
