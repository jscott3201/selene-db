#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_persist::AuditLog;

// Invariant: arbitrary bytes -> Ok or typed PersistError, never panic/OOM/hang.
// Exercises the SLAU file header then the record-scan loop: per-record 20-byte
// header decode, the MAX_AUDIT_PAYLOAD_BYTES cap, the payload-vs-remaining bound,
// the xxh3 checksum, and the torn-tail truncation (a garbage tail yields an
// Ok(truncated) vector, never a panic).
fuzz_target!(|bytes: &[u8]| {
    let _ = AuditLog::decode_all(bytes);
});
