#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_persist::SnapshotReader;

// Drives the snapshot envelope decode on attacker bytes: the 32-byte SLSN header
// (magic/version/flags/reserved/hash), the section-count*24-vs-remaining
// pre-allocation guard, the per-row payload-length cap, the duplicate-tag check,
// and the offset/overflow/EOF layout validation. The section *payload* rkyv
// decode is intentionally out of scope (it lives downstream in selene-graph
// behind a CheckBytes bound). Only a panic/OOM/hang fails.
fuzz_target!(|bytes: &[u8]| {
    let _ = SnapshotReader::decode_envelope(bytes);
});
