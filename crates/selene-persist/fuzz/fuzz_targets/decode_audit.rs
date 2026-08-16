#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_persist::AuditLog;

// Invariant: arbitrary bytes -> Ok or typed PersistError, never panic/OOM/hang.
// Exercises the SLAU file header then the record-scan loop: the per-record
// 24-byte header checksum, the decode behind it, the MAX_AUDIT_PAYLOAD_BYTES
// cap, the payload-vs-remaining bound, the xxh3 payload checksum, and the
// tail-vs-corruption classifier.
//
// Note what this target does NOT assert. Under v2 a garbage tail is far more
// likely to be classified as corruption (`AuditMidLogCorruption`) than silently
// truncated, so "Ok(truncated)" is no longer the expected shape and the useful
// invariant is only the absence of a panic. Reaching the deeper branches needs
// structurally valid headers with a correct checksum, which random bytes will
// essentially never produce — the same two-pass structured-synthesis treatment
// the WAL targets received would be required to fuzz past the first checksum.
fuzz_target!(|bytes: &[u8]| {
    let _ = AuditLog::decode_all(bytes);
});
