use selene_core::logical::Limits;
use selene_graph::logical_transaction::LogicalTransaction;
use selene_persist::{
    control::{StoreEpoch, StoreId},
    logical_frame::{self, Boundary, Compression, Context, Decoded},
    logical_stream::repair_fixture_integrity,
};

const LIMIT: usize = 1 << 20;

fn context() -> Context {
    Context {
        store: StoreId::from_bytes([0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1]).unwrap(),
        epoch: StoreEpoch::new(1).unwrap(),
        sequence: 1,
        segment: [1; 32],
        previous: [2; 32],
    }
}

pub fn drive(mut bytes: &[u8], boundary: Boundary) -> usize {
    let mut expected = context();
    let limits = Limits {
        bytes: LIMIT,
        allocation: 8 * LIMIT,
        items: 32_768,
        metadata: 1024,
        ..Limits::default()
    };
    while !bytes.is_empty() {
        match logical_frame::decode(bytes, expected, boundary, LIMIT) {
            Ok(Decoded::Complete {
                body,
                digest,
                consumed,
            }) => {
                assert!((logical_frame::FRAME_OVERHEAD..=bytes.len()).contains(&consumed));
                let _ = LogicalTransaction::decode(&body, limits);
                bytes = &bytes[consumed..];
                expected.sequence += 1;
                expected.previous = digest;
            }
            Ok(Decoded::Incomplete { needed }) => {
                assert_eq!(boundary, Boundary::UnsealedEnd);
                assert!(needed > bytes.len());
                break;
            }
            Err(_) => break,
        }
    }
    (expected.sequence - 1) as usize
}

pub fn synthesize(input: &[u8], mutate: bool) -> (Vec<u8>, usize) {
    let mut expected = context();
    let mut stream = Vec::new();
    // At most sixteen frames; Auto exercises both raw and valid bounded Zstd.
    for body in input.chunks(4096) {
        let mut frame = logical_frame::encode(body, expected, Compression::Auto, LIMIT).unwrap();
        if mutate && body.len() >= 3 {
            // Repair integrity after a header edit so version, extent, codec and
            // lineage errors do not hide behind the first checksum rejection.
            let offset = usize::from(body[0]) % (logical_frame::HEADER_LEN - 32);
            frame[offset] ^= body[1];
            repair_fixture_integrity(&mut frame, logical_frame::HEADER_LEN);
        }
        expected.previous = frame[frame.len() - 32..].try_into().unwrap();
        expected.sequence += 1;
        stream.extend_from_slice(&frame);
    }
    (stream, (expected.sequence - 1) as usize)
}
