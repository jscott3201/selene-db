//! Canonical form for procedure-pack manifest hashing.

use serde_json::Value;

use super::{BLAKE3_HASH_SENTINEL, ProcedurePackManifest};

/// Compute the canonical byte representation of `manifest` for hashing.
///
/// The manifest is cloned, `content_hash` is replaced with
/// [`BLAKE3_HASH_SENTINEL`], the result is converted through
/// [`serde_json::Value`] for alphabetical object-key ordering, then serialized
/// to compact JSON bytes.
pub(crate) fn canonical_bytes_for_hashing(manifest: &ProcedurePackManifest) -> Vec<u8> {
    let mut clone = manifest.clone();
    clone.content_hash = BLAKE3_HASH_SENTINEL.to_owned();
    let value: Value = serde_json::to_value(&clone).expect("typed manifest serializes to Value");
    serde_json::to_vec(&value).expect("Value serializes to bytes")
}

/// Lowercase hex encoding for manifest content hashes.
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
