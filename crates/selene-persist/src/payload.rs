//! WAL payload codec.

use selene_core::Change;

use crate::entry_header::{COMPRESS_THRESHOLD, FLAG_PAYLOAD_COMPRESSED, ensure_payload_len};
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
        (
            zstd::stream::encode_all(raw.as_slice(), 1)
                .map_err(|error| PersistError::Compression(error.to_string()))?,
            FLAG_PAYLOAD_COMPRESSED,
        )
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
        zstd::stream::decode_all(bytes)
            .map_err(|error| PersistError::Compression(error.to_string()))?
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
    let hash = blake3::hash(bytes);
    u32::from_le_bytes([
        hash.as_bytes()[0],
        hash.as_bytes()[1],
        hash.as_bytes()[2],
        hash.as_bytes()[3],
    ])
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

    fn change(bytes: impl Into<Vec<u8>>) -> Change {
        Change::IndexExtensionEvent {
            provider: provider("payload.provider"),
            payload: Arc::from(bytes.into()),
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

    proptest! {
        #[test]
        fn index_extension_event_payload_round_trips(payload in proptest::collection::vec(any::<u8>(), 0..4096)) {
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
