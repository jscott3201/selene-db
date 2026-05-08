//! Postcard WAL entry header.

use std::io::Read;
use std::sync::Arc;

use selene_core::{HlcTimestamp, Origin};
use serde::{Deserialize, Serialize};

use crate::{PersistError, PersistResult};

/// Maximum encoded payload bytes per WAL entry.
pub const MAX_WAL_ENTRY_BYTES: usize = 256 * 1024 * 1024;
/// Maximum caller-defined principal bytes per WAL entry.
pub const MAX_PRINCIPAL_BYTES: usize = 4096;
/// Payloads at or above this encoded length are zstd-compressed.
pub const COMPRESS_THRESHOLD: usize = 128;
/// Header flag bit indicating that the on-disk payload is zstd-compressed.
pub const FLAG_PAYLOAD_COMPRESSED: u8 = 0b0000_0001;

pub(crate) const HEADER_SCRATCH_BYTES: usize = MAX_PRINCIPAL_BYTES + 128;

/// Postcard-encoded append-only WAL entry header.
///
/// Field order is part of the v1 wire format. `principal` is intentionally
/// last per spec 04 section 3.2 and D12. Future fields append after it as
/// design intent, but any streaming-safe layout change still follows spec 04
/// section 7 version-bump rules.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WalEntryHeader {
    /// Encoded on-disk payload length in bytes.
    pub payload_len: u32,
    /// Lower 32 bits of `blake3(payload)`.
    pub checksum_lo: u32,
    /// Monotonic WAL sequence assigned by the writer.
    pub sequence: u64,
    /// NTP64 HLC seconds component.
    pub hlc_seconds: u64,
    /// NTP64 HLC subseconds component.
    pub hlc_subseconds: u32,
    /// Replication origin node ID, or `0` for local origin.
    pub origin_node_id: u64,
    /// Entry flags; bit 0 means the payload is zstd-compressed.
    pub flags: u8,
    /// Caller-owned opaque audit principal bytes, capped at 4 KiB.
    pub principal: Option<Arc<[u8]>>,
}

impl WalEntryHeader {
    /// Construct a validated v1 WAL entry header.
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
        let origin_node_id = match origin {
            Origin::Local => 0,
            Origin::Replicated { source_node_id, .. } => source_node_id.get(),
        };
        Ok(Self {
            payload_len: payload_len as u32,
            checksum_lo,
            sequence,
            hlc_seconds: hlc.seconds,
            hlc_subseconds: hlc.subseconds,
            origin_node_id,
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
    postcard::to_stdvec(header).map_err(|error| PersistError::HeaderCodec(error.to_string()))
}

pub(crate) fn read_entry_header<R: Read>(
    reader: R,
    offset: u64,
) -> PersistResult<(WalEntryHeader, R)> {
    let mut scratch = [0_u8; HEADER_SCRATCH_BYTES];
    match postcard::from_io::<WalEntryHeader, _>((reader, &mut scratch)) {
        Ok((header, (reader, _))) => Ok((header, reader)),
        Err(postcard::Error::DeserializeUnexpectedEnd) => {
            Err(PersistError::TruncatedEntry { offset })
        }
        Err(error) => Err(PersistError::HeaderCodec(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use selene_core::NodeId;

    fn principal(bytes: impl Into<Vec<u8>>) -> Arc<[u8]> {
        Arc::from(bytes.into())
    }

    #[test]
    fn local_header_round_trips() {
        let header =
            WalEntryHeader::new(17, 42, 3, HlcTimestamp::new(10, 20), Origin::Local, 0, None)
                .unwrap();
        let bytes = encode_entry_header(&header).unwrap();
        let (decoded, remainder) = read_entry_header(bytes.as_slice(), 0).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(remainder.len(), 0);
        assert_eq!(decoded.origin_node_id, 0);
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
        let (decoded, _) = read_entry_header(bytes.as_slice(), 0).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(decoded.origin_node_id, 55);
        assert!(decoded.is_payload_compressed());
        assert_eq!(decoded.principal.as_deref(), Some(b"alice".as_slice()));
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
            principal in proptest::option::of(proptest::collection::vec(any::<u8>(), 0..128)),
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
            let (decoded, _) = read_entry_header(bytes.as_slice(), 0).unwrap();
            let reencoded = encode_entry_header(&decoded).unwrap();
            prop_assert_eq!(decoded, header);
            prop_assert_eq!(reencoded, bytes);
        }
    }
}
