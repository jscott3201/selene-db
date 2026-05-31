//! WAL payload codec.

use selene_core::Change;

use crate::compression::{compress_zstd, decompress_zstd_bounded};
use crate::entry_header::{
    COMPRESS_THRESHOLD, FLAG_PAYLOAD_COMPRESSED, MAX_WAL_ENTRY_BYTES, ensure_payload_len,
};
use crate::{PersistError, PersistResult, WalEntryHeader};

pub(crate) struct EncodedPayload {
    pub(crate) bytes: Vec<u8>,
    pub(crate) flags: u8,
    pub(crate) checksum_lo: u32,
}

pub(crate) fn encode_changes(changes: &[Change]) -> PersistResult<EncodedPayload> {
    let raw = postcard::to_stdvec(changes)
        .map_err(|error| PersistError::PayloadCodec(error.to_string()))?;
    let (bytes, flags) = if raw.len() >= COMPRESS_THRESHOLD {
        (compress_zstd(raw.as_slice(), 1)?, FLAG_PAYLOAD_COMPRESSED)
    } else {
        (raw, 0)
    };
    ensure_payload_len(bytes.len())?;
    let checksum_lo = checksum_lo(&bytes);
    Ok(EncodedPayload {
        bytes,
        flags,
        checksum_lo,
    })
}

pub(crate) fn decode_changes(bytes: &[u8], compressed: bool) -> PersistResult<Vec<Change>> {
    let raw = if compressed {
        decompress_zstd_bounded(bytes, MAX_WAL_ENTRY_BYTES, |len, max| {
            PersistError::PayloadTooLarge { len, max }
        })?
    } else {
        bytes.to_vec()
    };
    postcard::from_bytes(&raw).map_err(|error| PersistError::PayloadCodec(error.to_string()))
}

pub(crate) fn verify_checksum(header: &WalEntryHeader, bytes: &[u8]) -> PersistResult<()> {
    if checksum_lo(bytes) != header.checksum_lo {
        return Err(PersistError::ChecksumMismatch {
            sequence: header.sequence,
        });
    }
    Ok(())
}

pub(crate) fn checksum_lo(bytes: &[u8]) -> u32 {
    let hash = xxhash_rust::xxh3::xxh3_64(bytes);
    (hash & 0xFFFF_FFFF) as u32
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;
    use selene_core::{Change, EdgeId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};

    use super::*;

    fn provider(name: &str) -> IStr {
        intern(name).unwrap()
    }

    // A `NodeCreated` carrying a `Value::Bytes` property is the byte-payload-
    // bearing change used to exercise the size-sensitive codec paths
    // (compression threshold, bounded decode). Its serialized footprint scales
    // with the supplied byte buffer just like the former extension-event payload.
    fn change(bytes: impl Into<Vec<u8>>) -> Change {
        Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(provider("payload.node")),
            properties: PropertyMap::from_pairs([(
                provider("payload.property"),
                Value::Bytes(Arc::from(bytes.into())),
            )])
            .unwrap(),
        }
    }

    fn change_strategy() -> impl Strategy<Value = Change> {
        let node_label = provider("payload.node");
        let edge_label = provider("payload.edge");
        let prop = provider("payload.property");
        prop_oneof![
            (1_u64..10_000).prop_map(move |id| Change::NodeCreated {
                id: NodeId::new(id),
                labels: LabelSet::single(node_label),
                properties: PropertyMap::from_pairs([(prop, Value::Int(id as i64))]).unwrap(),
            }),
            (1_u64..10_000).prop_map(|id| Change::NodeDeleted {
                id: NodeId::new(id),
            }),
            (1_u64..10_000).prop_map(move |id| Change::NodePropertyRemoved {
                id: NodeId::new(id),
                property: prop,
            }),
            (1_u64..10_000).prop_map(move |id| Change::NodeLabelRemoved {
                id: NodeId::new(id),
                label: node_label,
            }),
            (1_u64..10_000, 1_u64..10_000).prop_map(move |(id, target)| Change::EdgeCreated {
                id: EdgeId::new(id),
                label: edge_label,
                source: NodeId::new(id),
                target: NodeId::new(target),
                properties: PropertyMap::new(),
            }),
            (1_u64..10_000).prop_map(|id| Change::EdgeDeleted {
                id: EdgeId::new(id),
            }),
            (1_u64..10_000).prop_map(move |id| Change::EdgePropertyRemoved {
                id: EdgeId::new(id),
                property: prop,
            }),
            Just(Change::NodesOfTypeTruncated { label: node_label }),
            Just(Change::EdgesOfTypeTruncated { label: edge_label }),
            Just(Change::GraphReset {}),
            proptest::collection::vec(any::<u8>(), 0..512).prop_map(change),
        ]
    }

    #[test]
    fn small_payload_stays_uncompressed() {
        let changes = vec![change([1_u8, 2, 3])];
        let encoded = encode_changes(&changes).unwrap();
        assert_eq!(encoded.flags, 0);
        assert_eq!(decode_changes(&encoded.bytes, false).unwrap(), changes);
    }

    #[test]
    fn larger_payload_is_compressed() {
        let changes = vec![change(vec![7_u8; COMPRESS_THRESHOLD * 4])];
        let encoded = encode_changes(&changes).unwrap();
        assert_eq!(encoded.flags, FLAG_PAYLOAD_COMPRESSED);
        assert_eq!(decode_changes(&encoded.bytes, true).unwrap(), changes);
    }

    #[test]
    fn checksum_uses_on_disk_bytes() {
        let changes = vec![change(vec![8_u8; COMPRESS_THRESHOLD * 4])];
        let encoded = encode_changes(&changes).unwrap();
        assert_eq!(encoded.checksum_lo, checksum_lo(&encoded.bytes));
    }

    #[test]
    fn xxh3_checksum_is_stable() {
        assert_eq!(checksum_lo(b"selene"), 0xD795_2FA1);
    }

    #[test]
    fn checksum_mismatch_is_reported() {
        let changes = vec![change([1_u8])];
        let encoded = encode_changes(&changes).unwrap();
        let header = WalEntryHeader::new(
            encoded.bytes.len(),
            encoded.checksum_lo ^ 1,
            7,
            selene_core::HlcTimestamp::zero(),
            selene_core::Origin::Local,
            encoded.flags,
            None,
        )
        .unwrap();
        assert!(matches!(
            verify_checksum(&header, &encoded.bytes),
            Err(PersistError::ChecksumMismatch { sequence: 7 })
        ));
    }

    #[test]
    fn corrupt_compressed_bytes_report_compression_error() {
        let err = decode_changes(&[0, 1, 2, 3], true).unwrap_err();
        assert!(matches!(err, PersistError::Compression(_)));
    }

    #[test]
    fn bounded_decompress_rejects_oversized_output() {
        // Construct a small compressed payload that decompresses to
        // MAX_WAL_ENTRY_BYTES + 1, then assert decompress_bounded returns
        // PayloadTooLarge before runaway allocation. We use a highly-
        // compressible all-zero buffer so the compressed footprint stays
        // small while the decompressed footprint exceeds the cap.
        let oversize = MAX_WAL_ENTRY_BYTES + 1;
        let huge = vec![0_u8; oversize];
        let compressed =
            zstd::stream::encode_all(huge.as_slice(), 1).expect("zstd encode of test buffer");
        let err = decompress_zstd_bounded(&compressed, MAX_WAL_ENTRY_BYTES, |len, max| {
            PersistError::PayloadTooLarge { len, max }
        })
        .unwrap_err();
        match err {
            PersistError::PayloadTooLarge { len, max } => {
                assert!(len > max);
                assert_eq!(max, MAX_WAL_ENTRY_BYTES);
            }
            other => panic!("expected PayloadTooLarge, got {other:?}"),
        }
    }

    proptest! {
        #[test]
        fn byte_payload_change_round_trips(payload in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let changes = vec![change(payload)];
            let encoded = encode_changes(&changes).unwrap();
            let decoded = decode_changes(&encoded.bytes, encoded.flags & FLAG_PAYLOAD_COMPRESSED != 0).unwrap();
            prop_assert_eq!(decoded, changes);
        }

        #[test]
        fn mixed_change_vec_round_trips(changes in proptest::collection::vec(change_strategy(), 0..64)) {
            let encoded = encode_changes(&changes).unwrap();
            let decoded = decode_changes(&encoded.bytes, encoded.flags & FLAG_PAYLOAD_COMPRESSED != 0).unwrap();
            prop_assert_eq!(decoded, changes);
        }
    }
}
