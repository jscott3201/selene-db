//! Procedure-pack content hash placeholder.

use crate::ProcedurePackManifest;

/// Typed procedure-pack content hash.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Return the v1.0 placeholder hash.
    #[must_use]
    pub const fn placeholder() -> Self {
        Self([0_u8; 32])
    }

    /// Return true when this hash is the v1.0 placeholder.
    #[must_use]
    pub fn is_placeholder(self) -> bool {
        self.0 == [0_u8; 32]
    }

    /// Return this content hash as raw bytes.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Compute the typed content hash from a validated manifest.
    ///
    /// BRIEF-45 returns the placeholder unconditionally because
    /// [`parse_manifest`](crate::parse_manifest) already enforces the placeholder
    /// manifest string. BRIEF-48 will replace this body with canonical hashing.
    #[must_use]
    pub fn from_validated_manifest(_manifest: &ProcedurePackManifest) -> Self {
        Self::placeholder()
    }
}
