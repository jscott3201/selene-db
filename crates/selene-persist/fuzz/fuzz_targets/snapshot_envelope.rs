use selene_persist::{
    control::{StoreEpoch, StoreId},
    logical_snapshot::{self, SnapshotContext},
    logical_stream::Position,
};

pub fn context() -> SnapshotContext {
    SnapshotContext {
        boundary: Position {
            store: StoreId::from_bytes([0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1])
                .unwrap(),
            epoch: StoreEpoch::new(1).unwrap(),
            segment: [1; 32],
            sequence: 2,
            offset: 512,
            digest: [2; 32],
        },
        publication: 2,
    }
}

pub fn drive(bytes: &[u8]) {
    let expected = context();
    let _ = logical_snapshot::required_length(bytes, expected, 65_536);
    // Accept the trailer as a fixture digest only, not as selected control proof.
    // The decoder still verifies the complete envelope hash independently.
    let digest = bytes
        .get(bytes.len().saturating_sub(32)..)
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0; 32]);
    if let Ok(body) = logical_snapshot::decode(bytes, expected, &digest, 65_536) {
        assert_eq!(body.len() + logical_snapshot::OVERHEAD, bytes.len());
    }
    let _ = logical_snapshot::decode(bytes, expected, &[0; 32], 65_536);
}
