//! Procedure-pack content hash.

use crate::{ProcedurePackManifest, manifest::canonical_bytes_for_hashing};

/// Typed procedure-pack content hash.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Compute the content hash over canonical bytes.
    #[must_use]
    pub fn compute(canonical_bytes: &[u8]) -> Self {
        Self(blake3::hash(canonical_bytes).into())
    }

    /// Return this content hash as raw bytes.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Compute the typed content hash from a validated manifest.
    ///
    /// The manifest is re-canonicalized with its `content_hash` field set to
    /// the fixed sentinel before hashing.
    #[must_use]
    pub fn from_validated_manifest(manifest: &ProcedurePackManifest) -> Self {
        Self::compute(&canonical_bytes_for_hashing(manifest))
    }
}
