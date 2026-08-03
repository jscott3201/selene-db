//! Entry-header encode/decode, byte-layout pinning, and framing integrity.

use super::*;
use proptest::prelude::*;
use rstest::rstest;
use selene_core::NodeId;

fn principal(bytes: impl Into<Vec<u8>>) -> Arc<[u8]> {
    Arc::from(bytes.into())
}

fn read_from_bytes(bytes: &[u8]) -> PersistResult<(WalEntryHeader, usize, usize)> {
    let mut cursor = bytes;
    let (header, consumed) = read_entry_header(&mut cursor, 0)?;
    Ok((header, consumed, cursor.len()))
}

fn expected_len(header: &WalEntryHeader) -> usize {
    FIXED_ENTRY_HEADER_BYTES
        + match header.origin {
            Origin::Local => 0,
            Origin::Replicated { .. } => REPLICATED_TAIL_BYTES,
        }
        + header.principal.as_deref().map_or(0, <[u8]>::len)
}

#[test]
fn local_header_round_trips() {
    let header =
        WalEntryHeader::new(17, 42, 3, HlcTimestamp::new(10, 20), Origin::Local, 0, None).unwrap();
    let bytes = encode_entry_header(&header).unwrap();
    let (decoded, consumed, remainder) = read_from_bytes(&bytes).unwrap();
    assert_eq!(decoded, header);
    assert_eq!(consumed, FIXED_ENTRY_HEADER_BYTES);
    assert_eq!(remainder, 0);
    assert_eq!(decoded.origin, Origin::Local);
    assert_eq!(decoded.hlc(), HlcTimestamp::new(10, 20));
}

#[test]
fn replicated_header_round_trips_with_principal() {
    let header = WalEntryHeader::new(
        127,
        7,
        9,
        HlcTimestamp::new(1, 2),
        Origin::Replicated {
            source_node_id: NodeId::new(55),
            source_seq: 88,
        },
        FLAG_PAYLOAD_COMPRESSED,
        Some(principal(b"alice".to_vec())),
    )
    .unwrap();
    let bytes = encode_entry_header(&header).unwrap();
    let (decoded, consumed, _) = read_from_bytes(&bytes).unwrap();
    assert_eq!(decoded, header);
    assert_eq!(
        consumed,
        FIXED_ENTRY_HEADER_BYTES + REPLICATED_TAIL_BYTES + 5
    );
    assert_eq!(
        decoded.origin,
        Origin::Replicated {
            source_node_id: NodeId::new(55),
            source_seq: 88,
        }
    );
    assert!(decoded.is_payload_compressed());
    assert_eq!(decoded.principal.as_deref(), Some(b"alice".as_slice()));
}

#[test]
fn header_fixed_layout_byte_offsets() {
    let header = WalEntryHeader::new(
        0x0102_0304,
        0x0506_0708,
        0x1112_1314_1516_1718,
        HlcTimestamp::new(0x2122_2324_2526_2728, 0x3132_3334),
        Origin::Replicated {
            source_node_id: NodeId::new(0x4142_4344_4546_4748),
            source_seq: 0x5152_5354_5556_5758,
        },
        FLAG_PAYLOAD_COMPRESSED,
        Some(principal(b"abc".to_vec())),
    )
    .unwrap();
    let bytes = encode_entry_header(&header).unwrap();
    assert_eq!(
        bytes.len(),
        FIXED_ENTRY_HEADER_BYTES + REPLICATED_TAIL_BYTES + 3
    );

    // Both checksums are recomputed here from first principles rather than
    // through the encoder's helper, so a change to what the checksums cover
    // fails this test instead of moving with it.
    let mut extent = Vec::new();
    extent.extend_from_slice(&0x4142_4344_4546_4748_u64.to_le_bytes());
    extent.extend_from_slice(&0x5152_5354_5556_5758_u64.to_le_bytes());
    extent.extend_from_slice(b"abc");
    let expected_extent = xxhash_rust::xxh3::xxh3_64(&extent) as u32;
    let expected_prefix = xxhash_rust::xxh3::xxh3_64(&bytes[4..40]) as u32;

    assert_eq!(
        &bytes[0..4],
        &expected_prefix.to_le_bytes(),
        "prefix_checksum"
    );
    assert_eq!(&bytes[4..8], &0x0102_0304_u32.to_le_bytes(), "payload_len");
    assert_eq!(
        &bytes[8..12],
        &0x0506_0708_u32.to_le_bytes(),
        "payload checksum"
    );
    assert_eq!(
        &bytes[12..20],
        &0x1112_1314_1516_1718_u64.to_le_bytes(),
        "sequence"
    );
    assert_eq!(
        &bytes[20..28],
        &0x2122_2324_2526_2728_u64.to_le_bytes(),
        "hlc_seconds"
    );
    assert_eq!(
        &bytes[28..32],
        &0x3132_3334_u32.to_le_bytes(),
        "hlc_subseconds"
    );
    assert_eq!(
        bytes[32],
        FLAG_PAYLOAD_COMPRESSED | FLAG_REPLICATED_ORIGIN,
        "flags: the replicated origin is a flag bit in v3, not a tag byte"
    );
    assert_eq!(bytes[33], 0, "reserved");
    assert_eq!(&bytes[34..36], &4_u16.to_le_bytes(), "principal_len + 1");
    assert_eq!(
        &bytes[36..40],
        &expected_extent.to_le_bytes(),
        "extent_checksum"
    );
    assert_eq!(&bytes[40..48], &0x4142_4344_4546_4748_u64.to_le_bytes());
    assert_eq!(&bytes[48..56], &0x5152_5354_5556_5758_u64.to_le_bytes());
    assert_eq!(&bytes[56..59], b"abc");
}

#[test]
fn principal_length_round_trips_none_empty_some() {
    for principal in [
        None,
        Some(Arc::from([])),
        Some(Arc::from(b"alice".as_slice())),
    ] {
        let header = WalEntryHeader::new(
            1,
            2,
            3,
            HlcTimestamp::zero(),
            Origin::Local,
            0,
            principal.clone(),
        )
        .unwrap();
        let bytes = encode_entry_header(&header).unwrap();
        let (decoded, consumed, _) = read_from_bytes(&bytes).unwrap();
        assert_eq!(decoded.principal, principal);
        assert_eq!(decoded, header);
        assert_eq!(consumed, expected_len(&header));
    }
}

#[test]
fn truncated_fixed_prefix_returns_truncated_entry() {
    let bytes = [0_u8; 16];
    let mut cursor = bytes.as_slice();
    assert!(matches!(
        read_entry_header(&mut cursor, 99),
        Err(PersistError::TruncatedEntry { offset: 99 })
    ));
}

#[test]
fn read_entry_header_returns_bytes_consumed() {
    let header = WalEntryHeader::new(
        1,
        2,
        3,
        HlcTimestamp::zero(),
        Origin::Replicated {
            source_node_id: NodeId::new(4),
            source_seq: 5,
        },
        0,
        Some(principal(b"alice".to_vec())),
    )
    .unwrap();
    let bytes = encode_entry_header(&header).unwrap();
    let (decoded, consumed, _) = read_from_bytes(&bytes).unwrap();
    assert_eq!(decoded, header);
    assert_eq!(consumed, bytes.len());
}

/// The #1090 regression. Every one of these decoded cleanly before the
/// prefix checksum existed: the payload checksum covered only the payload,
/// so a flip in the timestamp replayed with the wrong HLC, a flip in the
/// principal replayed under the wrong audit identity, and a flip in a
/// length field was used as a file offset.
#[rstest]
#[case::payload_len(4)]
#[case::payload_checksum(8)]
#[case::sequence(12)]
#[case::hlc_seconds(20)]
#[case::hlc_subseconds(28)]
#[case::principal_len(34)]
#[case::extent_checksum(36)]
#[case::provenance_source_node(40)]
#[case::provenance_source_seq(48)]
#[case::principal_bytes(56)]
fn framing_corruption_is_rejected(#[case] index: usize) {
    let header = WalEntryHeader::new(
        17,
        42,
        3,
        HlcTimestamp::new(10, 20),
        Origin::Replicated {
            source_node_id: NodeId::new(9),
            source_seq: 4,
        },
        0,
        Some(principal(b"alice".to_vec())),
    )
    .unwrap();
    let mut bytes = encode_entry_header(&header).unwrap();
    bytes[index] ^= 0b0000_0001;
    let mut cursor = bytes.as_slice();
    let observed = read_entry_header(&mut cursor, 0);
    assert!(
        matches!(
            observed,
            Err(PersistError::WalHeaderChecksumMismatch { .. })
        ),
        "byte {index} must be under integrity protection, got {:?}",
        observed.map(|(header, _)| header)
    );
}

/// v2 spent a whole byte on `origin_tag`, giving it 254 invalid values that
/// had to be rejected before the frame length was known. In v3 the origin
/// is a flag bit, so every bit pattern yields a well-defined length and the
/// prefix checksum is the sole authority — a flipped bit here is a checksum
/// failure, not a bespoke decode error.
#[test]
fn corrupt_origin_bit_fails_the_prefix_checksum() {
    let header =
        WalEntryHeader::new(17, 42, 3, HlcTimestamp::new(10, 20), Origin::Local, 0, None).unwrap();
    let mut bytes = encode_entry_header(&header).unwrap();
    bytes[32] ^= FLAG_REPLICATED_ORIGIN;
    let mut cursor = bytes.as_slice();
    assert!(matches!(
        read_entry_header(&mut cursor, 7),
        Err(PersistError::WalHeaderChecksumMismatch { offset: 7 })
    ));
}

/// An undefined flag bit is rejected at both ends: a producer cannot write
/// a frame this version cannot describe, and a reader will not guess.
#[test]
fn undefined_flag_bits_are_rejected() {
    let header =
        WalEntryHeader::new(17, 42, 3, HlcTimestamp::new(10, 20), Origin::Local, 0, None).unwrap();
    let bytes = encode_entry_header(&header).unwrap();
    // Reader side: set an undefined bit and repair the checksum, so the
    // frame is internally consistent and only the flag is objectionable.
    let mut corrupted = bytes.clone();
    corrupted[32] |= 0b1000_0000;
    let checksum = crate::payload::checksum_lo(&corrupted[4..40]);
    corrupted[0..4].copy_from_slice(&checksum.to_le_bytes());
    let mut cursor = corrupted.as_slice();
    assert!(matches!(
        read_entry_header(&mut cursor, 0),
        Err(PersistError::UnsupportedFlag { flag: 0b1000_0000 })
    ));

    // Writer side.
    let rejected = WalEntryHeader::new(
        17,
        42,
        3,
        HlcTimestamp::new(10, 20),
        Origin::Local,
        0b1000_0000,
        None,
    )
    .unwrap();
    assert!(matches!(
        encode_entry_header(&rejected),
        Err(PersistError::UnsupportedFlag { flag: 0b1000_0000 })
    ));
}

#[test]
fn principal_cap_boundary_is_allowed() {
    let bytes = vec![1_u8; MAX_PRINCIPAL_BYTES];
    let header = WalEntryHeader::new(
        0,
        0,
        1,
        HlcTimestamp::zero(),
        Origin::Local,
        0,
        Some(principal(bytes.clone())),
    )
    .unwrap();
    assert_eq!(header.principal.unwrap().len(), bytes.len());
}

#[test]
fn principal_overflow_is_rejected() {
    let err = WalEntryHeader::new(
        0,
        0,
        1,
        HlcTimestamp::zero(),
        Origin::Local,
        0,
        Some(principal(vec![1_u8; MAX_PRINCIPAL_BYTES + 1])),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        PersistError::PrincipalTooLarge {
            len,
            max: MAX_PRINCIPAL_BYTES
        } if len == MAX_PRINCIPAL_BYTES + 1
    ));
}

#[test]
fn payload_at_cap_is_accepted() {
    // The +1 rejection has a test; pin the accept-at-cap twin. Target
    // `WalEntryHeader::new` (which calls `ensure_payload_len`) with the cap
    // value directly so no 256 MiB buffer is allocated.
    ensure_payload_len(MAX_WAL_ENTRY_BYTES).expect("cap-length payload is accepted");
    let header = WalEntryHeader::new(
        MAX_WAL_ENTRY_BYTES,
        0,
        1,
        HlcTimestamp::zero(),
        Origin::Local,
        0,
        None,
    )
    .expect("WalEntryHeader::new accepts a cap-length payload");
    assert_eq!(header.payload_len as usize, MAX_WAL_ENTRY_BYTES);
}

#[test]
fn payload_cap_is_enforced() {
    let err = WalEntryHeader::new(
        MAX_WAL_ENTRY_BYTES + 1,
        0,
        1,
        HlcTimestamp::zero(),
        Origin::Local,
        0,
        None,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        PersistError::PayloadTooLarge {
            len,
            max: MAX_WAL_ENTRY_BYTES
        } if len == MAX_WAL_ENTRY_BYTES + 1
    ));
}

proptest! {
    #[test]
    fn header_field_permutations_round_trip_bit_identically(
        payload_len in 0_usize..8192,
        checksum_lo in any::<u32>(),
        sequence in 1_u64..10_000,
        seconds in any::<u64>(),
        subseconds in any::<u32>(),
        origin_node_id in 0_u64..10_000,
        compressed in any::<bool>(),
        principal in prop_oneof![
            Just(None),
            Just(Some(Vec::new())),
            proptest::collection::vec(any::<u8>(), 1..128).prop_map(Some),
        ],
    ) {
        let origin = if origin_node_id == 0 {
            Origin::Local
        } else {
            Origin::Replicated {
                source_node_id: NodeId::new(origin_node_id),
                source_seq: sequence,
            }
        };
        let flags = if compressed { FLAG_PAYLOAD_COMPRESSED } else { 0 };
        let header = WalEntryHeader::new(
            payload_len,
            checksum_lo,
            sequence,
            HlcTimestamp::new(seconds, subseconds),
            origin,
            flags,
            principal.map(Arc::from),
        )
        .unwrap();
        let bytes = encode_entry_header(&header).unwrap();
        prop_assert_eq!(bytes.len(), expected_len(&header));
        let (decoded, consumed, _) = read_from_bytes(&bytes).unwrap();
        let reencoded = encode_entry_header(&decoded).unwrap();
        prop_assert_eq!(consumed, bytes.len());
        prop_assert_eq!(decoded, header);
        prop_assert_eq!(reencoded, bytes);
    }
}
