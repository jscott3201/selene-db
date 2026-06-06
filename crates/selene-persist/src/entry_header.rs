//! Fixed-layout WAL entry header.

use std::io::{ErrorKind, Read};
use std::sync::Arc;

use selene_core::{HlcTimestamp, NodeId, Origin};

use crate::{PersistError, PersistResult};

/// Maximum encoded payload bytes per WAL entry.
pub const MAX_WAL_ENTRY_BYTES: usize = 256 * 1024 * 1024;
/// Maximum caller-defined principal bytes per WAL entry.
pub const MAX_PRINCIPAL_BYTES: usize = 4096;
/// Payloads at or above this encoded length are zstd-compressed.
pub const COMPRESS_THRESHOLD: usize = 4_096;
/// Header flag bit indicating that the on-disk payload is zstd-compressed.
pub const FLAG_PAYLOAD_COMPRESSED: u8 = 0b0000_0001;

pub(crate) const FIXED_ENTRY_HEADER_BYTES: usize = 32;
const REPLICATED_TAIL_BYTES: usize = 16;
const MAX_ENTRY_HEADER_BYTES: usize =
    FIXED_ENTRY_HEADER_BYTES + REPLICATED_TAIL_BYTES + MAX_PRINCIPAL_BYTES;

const ORIGIN_TAG_LOCAL: u8 = 0;
const ORIGIN_TAG_REPLICATED: u8 = 1;

/// Fixed-layout append-only WAL entry header.
///
/// The v2 wire prefix is 32 bytes:
/// payload length, checksum, sequence, HLC seconds/subseconds, flags,
/// origin tag, and `principal_len + 1`. Replicated origins append a 16-byte
/// provenance tail; principals append their raw bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WalEntryHeader {
    /// Encoded on-disk payload length in bytes.
    pub payload_len: u32,
    /// Lower 32 bits of `xxh3_64(payload)`.
    pub checksum_lo: u32,
    /// Monotonic WAL sequence assigned by the writer.
    pub sequence: u64,
    /// NTP64 HLC seconds component.
    pub hlc_seconds: u64,
    /// NTP64 HLC subseconds component.
    pub hlc_subseconds: u32,
    /// Mutation origin (Local or Replicated with full provenance).
    pub origin: Origin,
    /// Entry flags; bit 0 means the payload is zstd-compressed.
    pub flags: u8,
    /// Caller-owned opaque audit principal bytes, capped at 4 KiB.
    pub principal: Option<Arc<[u8]>>,
}

impl WalEntryHeader {
    /// Construct a validated v2 WAL entry header.
    ///
    /// # Errors
    ///
    /// Returns [`PersistError::PayloadTooLarge`] if `payload_len` exceeds the
    /// 256 MiB cap, or [`PersistError::PrincipalTooLarge`] if the principal is
    /// larger than 4 KiB.
    pub fn new(
        payload_len: usize,
        checksum_lo: u32,
        sequence: u64,
        hlc: HlcTimestamp,
        origin: Origin,
        flags: u8,
        principal: Option<Arc<[u8]>>,
    ) -> PersistResult<Self> {
        ensure_payload_len(payload_len)?;
        validate_principal(principal.as_deref())?;
        Ok(Self {
            payload_len: payload_len as u32,
            checksum_lo,
            sequence,
            hlc_seconds: hlc.seconds,
            hlc_subseconds: hlc.subseconds,
            origin,
            flags,
            principal,
        })
    }

    /// Return the HLC timestamp carried by this header.
    #[must_use]
    pub const fn hlc(&self) -> HlcTimestamp {
        HlcTimestamp::new(self.hlc_seconds, self.hlc_subseconds)
    }

    /// Return true if the payload-compressed flag is set.
    #[must_use]
    pub const fn is_payload_compressed(&self) -> bool {
        self.flags & FLAG_PAYLOAD_COMPRESSED != 0
    }
}

pub(crate) fn ensure_payload_len(len: usize) -> PersistResult<()> {
    if len > MAX_WAL_ENTRY_BYTES {
        return Err(PersistError::PayloadTooLarge {
            len,
            max: MAX_WAL_ENTRY_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn validate_principal(principal: Option<&[u8]>) -> PersistResult<()> {
    if let Some(principal) = principal
        && principal.len() > MAX_PRINCIPAL_BYTES
    {
        return Err(PersistError::PrincipalTooLarge {
            len: principal.len(),
            max: MAX_PRINCIPAL_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn encode_entry_header(header: &WalEntryHeader) -> PersistResult<Vec<u8>> {
    ensure_payload_len(header.payload_len as usize)?;
    validate_principal(header.principal.as_deref())?;

    let principal_len = header.principal.as_deref().map_or(0, <[u8]>::len);
    let principal_len_p1 = header
        .principal
        .as_ref()
        .map(|_| u16::try_from(principal_len + 1).expect("principal cap fits in u16"))
        .unwrap_or(0);
    let origin_tail_len = match header.origin {
        Origin::Local => 0,
        Origin::Replicated { .. } => REPLICATED_TAIL_BYTES,
    };
    let mut bytes = Vec::with_capacity(MAX_ENTRY_HEADER_BYTES);

    bytes.extend_from_slice(&header.payload_len.to_le_bytes());
    bytes.extend_from_slice(&header.checksum_lo.to_le_bytes());
    bytes.extend_from_slice(&header.sequence.to_le_bytes());
    bytes.extend_from_slice(&header.hlc_seconds.to_le_bytes());
    bytes.extend_from_slice(&header.hlc_subseconds.to_le_bytes());
    bytes.push(header.flags);
    match header.origin {
        Origin::Local => bytes.push(ORIGIN_TAG_LOCAL),
        Origin::Replicated {
            source_node_id,
            source_seq,
        } => {
            bytes.push(ORIGIN_TAG_REPLICATED);
            bytes.extend_from_slice(&principal_len_p1.to_le_bytes());
            bytes.extend_from_slice(&source_node_id.get().to_le_bytes());
            bytes.extend_from_slice(&source_seq.to_le_bytes());
            if let Some(principal) = header.principal.as_deref() {
                bytes.extend_from_slice(principal);
            }
            debug_assert_eq!(
                bytes.len(),
                FIXED_ENTRY_HEADER_BYTES + origin_tail_len + principal_len
            );
            return Ok(bytes);
        }
    }
    bytes.extend_from_slice(&principal_len_p1.to_le_bytes());
    if let Some(principal) = header.principal.as_deref() {
        bytes.extend_from_slice(principal);
    }
    debug_assert_eq!(
        bytes.len(),
        FIXED_ENTRY_HEADER_BYTES + origin_tail_len + principal_len
    );
    Ok(bytes)
}

pub(crate) fn read_entry_header<R: Read>(
    reader: &mut R,
    offset: u64,
) -> PersistResult<(WalEntryHeader, usize)> {
    let mut fixed = [0_u8; FIXED_ENTRY_HEADER_BYTES];
    read_exact_header(reader, &mut fixed, offset)?;

    let payload_len = u32::from_le_bytes(fixed[0..4].try_into().expect("fixed header payload_len"));
    let checksum_lo = u32::from_le_bytes(fixed[4..8].try_into().expect("fixed header checksum_lo"));
    let sequence = u64::from_le_bytes(fixed[8..16].try_into().expect("fixed header sequence"));
    let hlc_seconds =
        u64::from_le_bytes(fixed[16..24].try_into().expect("fixed header hlc_seconds"));
    let hlc_subseconds = u32::from_le_bytes(
        fixed[24..28]
            .try_into()
            .expect("fixed header hlc_subseconds"),
    );
    let flags = fixed[28];
    let origin_tag = fixed[29];
    let principal_len_p1 = u16::from_le_bytes(
        fixed[30..32]
            .try_into()
            .expect("fixed header principal_len"),
    );

    ensure_payload_len(payload_len as usize)?;
    let principal_len = principal_len_p1.saturating_sub(1) as usize;
    if principal_len > MAX_PRINCIPAL_BYTES {
        return Err(PersistError::PrincipalTooLarge {
            len: principal_len,
            max: MAX_PRINCIPAL_BYTES,
        });
    }

    let (origin, origin_tail_len) = match origin_tag {
        ORIGIN_TAG_LOCAL => (Origin::Local, 0),
        ORIGIN_TAG_REPLICATED => {
            let mut tail = [0_u8; REPLICATED_TAIL_BYTES];
            read_exact_header(reader, &mut tail, offset)?;
            let source_node_id = u64::from_le_bytes(
                tail[0..8]
                    .try_into()
                    .expect("replicated tail source_node_id"),
            );
            let source_seq =
                u64::from_le_bytes(tail[8..16].try_into().expect("replicated tail source_seq"));
            (
                Origin::Replicated {
                    source_node_id: NodeId::new(source_node_id),
                    source_seq,
                },
                REPLICATED_TAIL_BYTES,
            )
        }
        other => {
            return Err(PersistError::HeaderCodec(format!(
                "unknown origin_tag {other}"
            )));
        }
    };

    let principal = if principal_len_p1 == 0 {
        None
    } else if principal_len == 0 {
        Some(Arc::from([]))
    } else {
        let mut principal = vec![0_u8; principal_len];
        read_exact_header(reader, &mut principal, offset)?;
        Some(Arc::from(principal))
    };

    Ok((
        WalEntryHeader {
            payload_len,
            checksum_lo,
            sequence,
            hlc_seconds,
            hlc_subseconds,
            origin,
            flags,
            principal,
        },
        FIXED_ENTRY_HEADER_BYTES + origin_tail_len + principal_len,
    ))
}

fn read_exact_header(reader: &mut impl Read, buf: &mut [u8], offset: u64) -> PersistResult<()> {
    reader.read_exact(buf).map_err(|error| match error.kind() {
        ErrorKind::UnexpectedEof => PersistError::TruncatedEntry { offset },
        _ => PersistError::Io(error),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
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
            WalEntryHeader::new(17, 42, 3, HlcTimestamp::new(10, 20), Origin::Local, 0, None)
                .unwrap();
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
        assert_eq!(&bytes[0..4], &0x0102_0304_u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &0x0506_0708_u32.to_le_bytes());
        assert_eq!(&bytes[8..16], &0x1112_1314_1516_1718_u64.to_le_bytes());
        assert_eq!(&bytes[16..24], &0x2122_2324_2526_2728_u64.to_le_bytes());
        assert_eq!(&bytes[24..28], &0x3132_3334_u32.to_le_bytes());
        assert_eq!(bytes[28], FLAG_PAYLOAD_COMPRESSED);
        assert_eq!(bytes[29], ORIGIN_TAG_REPLICATED);
        assert_eq!(&bytes[30..32], &4_u16.to_le_bytes());
        assert_eq!(&bytes[32..40], &0x4142_4344_4546_4748_u64.to_le_bytes());
        assert_eq!(&bytes[40..48], &0x5152_5354_5556_5758_u64.to_le_bytes());
        assert_eq!(&bytes[48..51], b"abc");
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

    #[test]
    fn unknown_origin_tag_is_rejected() {
        let header =
            WalEntryHeader::new(17, 42, 3, HlcTimestamp::new(10, 20), Origin::Local, 0, None)
                .unwrap();
        let mut bytes = encode_entry_header(&header).unwrap();
        bytes[29] = 9;
        let mut cursor = bytes.as_slice();
        assert!(matches!(
            read_entry_header(&mut cursor, 0),
            Err(PersistError::HeaderCodec(message)) if message.contains("unknown origin_tag 9")
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
}
