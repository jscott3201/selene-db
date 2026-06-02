#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_persist::Manifest;

// Invariant: arbitrary bytes -> Ok or typed PersistError, never panic/OOM/hang.
// Exercises the SLMF header (length guard, magic, version, reserved=0), the
// blake3 body hash, the checked LE field reads, and the capped postcard
// `Vec<u64>` archive-set tail.
fuzz_target!(|bytes: &[u8]| {
    let _ = Manifest::decode(bytes);
});
