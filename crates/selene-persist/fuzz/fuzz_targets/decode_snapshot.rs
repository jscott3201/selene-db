#![no_main]

use libfuzzer_sys::fuzz_target;
use selene_persist::{logical_snapshot, logical_stream::repair_fixture_integrity};
mod snapshot_envelope;

// Format-2 envelope coverage; semantic sections have decode_logical_snapshot.
// Context is supplied independently, never inferred as authority from input bytes.
fuzz_target!(|input: &[u8]| {
    if input.len() > 65_536 {
        return;
    }
    snapshot_envelope::drive(input);
    let mut frame = logical_snapshot::encode(input, snapshot_envelope::context(), 65_536).unwrap();
    let digest = frame[frame.len() - 32..].try_into().unwrap();
    assert_eq!(
        logical_snapshot::decode(&frame, snapshot_envelope::context(), &digest, 65_536).unwrap(),
        input
    );
    for edit in input.chunks_exact(3).take(32) {
        let index = usize::from(u16::from_le_bytes([edit[0], edit[1]])) % frame.len();
        frame[index] ^= edit[2];
    }
    snapshot_envelope::drive(&frame);
    repair_fixture_integrity(&mut frame, logical_snapshot::HEADER_LEN);
    snapshot_envelope::drive(&frame);
    let cut = input.get(..2).map_or(0, |b| {
        usize::from(u16::from_le_bytes([b[0], b[1]])) % frame.len()
    });
    snapshot_envelope::drive(&frame[..cut]);
    frame.push(0);
    snapshot_envelope::drive(&frame);
});
