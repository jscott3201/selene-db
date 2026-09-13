#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_persist::logical_frame::Boundary;
mod wal_stream;

// Pure format-2 stream consumption: no filesystem selection, truncation or repair.
// Structured frames keep payload decoding and digest/sequence advancement reachable.
fuzz_target!(|input: &[u8]| {
    if input.len() > 65_536 {
        return;
    }
    let (stream, count) = wal_stream::synthesize(input, false);
    let (mutated, _) = wal_stream::synthesize(input, true);
    let cut = input.get(..2).map_or(0, |b| {
        usize::from(u16::from_le_bytes([b[0], b[1]])) % (stream.len() + 1)
    });
    for boundary in [
        Boundary::UnsealedEnd,
        Boundary::SealedEnd,
        Boundary::Interior,
    ] {
        wal_stream::drive(input, boundary);
        assert_eq!(wal_stream::drive(&stream, boundary), count);
        wal_stream::drive(&stream[..cut], boundary);
        wal_stream::drive(&mutated, boundary);
    }
});
