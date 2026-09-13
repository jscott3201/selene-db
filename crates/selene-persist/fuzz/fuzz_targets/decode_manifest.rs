#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_persist::{
    control::{EmptyManifest, frame_fuzz_payload},
    logical_stream::{rotating_manifest_fuzz_payload, validate_manifest},
};

// Invariant: arbitrary bytes -> Ok or typed PersistError, never panic/OOM/hang.
// Exercise current format-2 empty, single-segment, checkpoint and rotating
// manifests. Pure validation grants no filesystem selection or recovery authority.
fuzz_target!(|bytes: &[u8]| {
    let _ = EmptyManifest::decode(bytes);
    let _ = validate_manifest(bytes);
    // Repair framing integrity so arbitrary bodies reach bounded postcard and
    // identity/lineage/layout validation instead of stopping at the BLAKE3 hash.
    for magic in [*b"SLEM", *b"SLLM", *b"SLDM", *b"SLRM"] {
        if let Ok(framed) = frame_fuzz_payload(bytes, magic) {
            let _ = EmptyManifest::decode(&framed);
            let _ = validate_manifest(&framed);
        }
    }
    // A valid selected-layout seed reaches deep rotating-manifest checks even
    // when arbitrary bodies cannot form the required nested metadata.
    let mut body = rotating_manifest_fuzz_payload();
    for edit in bytes.chunks_exact(3).take(32) {
        let index = usize::from(u16::from_le_bytes([edit[0], edit[1]])) % body.len();
        body[index] ^= edit[2];
    }
    let framed = frame_fuzz_payload(&body, *b"SLRM").unwrap();
    let _ = validate_manifest(&framed);
});
