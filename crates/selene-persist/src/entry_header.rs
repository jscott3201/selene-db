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
/// Header flag bit identifying a physical checkpoint-sequence watermark.
///
/// Watermarks carry a local, principal-free, empty change payload. They advance
/// the WAL sequence without representing a logical graph commit.
pub const FLAG_CHECKPOINT_WATERMARK: u8 = 0b0000_0010;
/// Header flag bit indicating that a 16-byte replicated provenance tail follows.
///
/// This replaces the v2 `origin_tag` byte. A tag byte had 254 invalid values
/// that had to be rejected *before* the frame length was known, which is
/// exactly the pre-checksum rejection the v3 layout exists to remove: as a flag
/// bit every pattern yields a well-defined length, so the prefix checksum is
/// the sole authority over this byte.
pub const FLAG_REPLICATED_ORIGIN: u8 = 0b0000_0100;

/// Flag bits this format version defines. Anything else is rejected.
const KNOWN_FLAGS: u8 =
    FLAG_PAYLOAD_COMPRESSED | FLAG_CHECKPOINT_WATERMARK | FLAG_REPLICATED_ORIGIN;

pub(crate) const FIXED_ENTRY_HEADER_BYTES: usize = 40;
const REPLICATED_TAIL_BYTES: usize = 16;
const MAX_ENTRY_HEADER_BYTES: usize =
    FIXED_ENTRY_HEADER_BYTES + REPLICATED_TAIL_BYTES + MAX_PRINCIPAL_BYTES;

/// Prefix bytes covered by `prefix_checksum`: everything after it.
///
/// The checksum sits at offset 0 so the covered range is a suffix, which means
/// no field has to be zeroed and no scratch copy is made to verify it.
const PREFIX_CHECKSUM_COVERAGE: std::ops::Range<usize> = 4..FIXED_ENTRY_HEADER_BYTES;

/// Fixed-layout append-only WAL entry header.
///
/// The v3 wire prefix is 40 bytes, little-endian throughout:
///
/// | range | field |
/// |---|---|
/// | `[0..4)` | prefix checksum over `[4..40)` |
/// | `[4..8)` | payload length |
/// | `[8..12)` | payload checksum |
/// | `[12..20)` | sequence |
/// | `[20..28)` | HLC seconds |
/// | `[28..32)` | HLC subseconds |
/// | `[32]` | flags |
/// | `[33]` | reserved, must be zero |
/// | `[34..36)` | `principal_len + 1`, or 0 for no principal |
/// | `[36..40)` | extent checksum |
///
/// The prefix is followed by the extent — a 16-byte provenance tail when
/// [`FLAG_REPLICATED_ORIGIN`] is set, then the principal bytes — and then the
/// payload. Both checksums are the low 32 bits of xxh3-64.
///
/// The prefix checksum sits at offset 0 so its coverage is a suffix: nothing has
/// to be zeroed and no scratch copy is made to verify it.
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
    /// Entry flags: bit 0 payload compression, bit 1 checkpoint watermark.
    ///
    /// Bit 2 ([`FLAG_REPLICATED_ORIGIN`]) is a wire-only bit. It is stripped on
    /// decode because the same information is already carried structurally by
    /// [`WalEntryHeader::origin`], and leaving both would let them disagree.
    pub flags: u8,
    /// Caller-owned opaque audit principal bytes, capped at 4 KiB.
    pub principal: Option<Arc<[u8]>>,
}

impl WalEntryHeader {
    /// Construct a validated WAL entry header.
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

    /// Return true when this frame is a physical checkpoint watermark.
    #[must_use]
    pub const fn is_checkpoint_watermark(&self) -> bool {
        self.flags & FLAG_CHECKPOINT_WATERMARK != 0
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
    if header.flags & !KNOWN_FLAGS != 0 {
        return Err(PersistError::UnsupportedFlag {
            flag: u16::from(header.flags & !KNOWN_FLAGS),
        });
    }

    let principal = header.principal.as_deref();
    let principal_len = principal.map_or(0, <[u8]>::len);
    let principal_len_p1 = principal
        .map(|_| u16::try_from(principal_len + 1).expect("principal cap fits in u16"))
        .unwrap_or(0);

    // The extent is everything the prefix's lengths describe: the optional
    // provenance tail followed by the principal. Hashed as one run so the
    // prefix carries a single field authenticating both.
    let mut extent = Vec::with_capacity(REPLICATED_TAIL_BYTES + principal_len);
    let mut flags = header.flags;
    match header.origin {
        Origin::Local => flags &= !FLAG_REPLICATED_ORIGIN,
        Origin::Replicated {
            source_node_id,
            source_seq,
        } => {
            flags |= FLAG_REPLICATED_ORIGIN;
            extent.extend_from_slice(&source_node_id.get().to_le_bytes());
            extent.extend_from_slice(&source_seq.to_le_bytes());
        }
    }
    if let Some(principal) = principal {
        extent.extend_from_slice(principal);
    }

    let mut bytes = Vec::with_capacity(MAX_ENTRY_HEADER_BYTES);
    bytes.extend_from_slice(&0_u32.to_le_bytes()); // prefix_checksum, backfilled below
    bytes.extend_from_slice(&header.payload_len.to_le_bytes());
    bytes.extend_from_slice(&header.checksum_lo.to_le_bytes());
    bytes.extend_from_slice(&header.sequence.to_le_bytes());
    bytes.extend_from_slice(&header.hlc_seconds.to_le_bytes());
    bytes.extend_from_slice(&header.hlc_subseconds.to_le_bytes());
    bytes.push(flags);
    bytes.push(0); // reserved
    bytes.extend_from_slice(&principal_len_p1.to_le_bytes());
    bytes.extend_from_slice(&crate::payload::checksum_lo(&extent).to_le_bytes());
    debug_assert_eq!(bytes.len(), FIXED_ENTRY_HEADER_BYTES);

    // Backfilled on the single exit path. A per-origin early return here would
    // produce frames that fail their own verification for one origin only.
    let prefix_checksum = crate::payload::checksum_lo(&bytes[PREFIX_CHECKSUM_COVERAGE]);
    bytes[0..4].copy_from_slice(&prefix_checksum.to_le_bytes());

    bytes.extend_from_slice(&extent);
    debug_assert_eq!(bytes.len(), FIXED_ENTRY_HEADER_BYTES + extent.len());
    Ok(bytes)
}

/// A frame prefix whose checksum has been verified.
///
/// Existing only in the verified state is the point: it is produced solely by
/// [`read_frame_prefix`], which returns it only after the prefix checksum
/// passes. Holding one is therefore proof that the lengths inside it can be
/// trusted as read lengths and file offsets — which is what lets the tail scan
/// tell a torn tail from corruption by asking where the frame ends.
pub(crate) struct FramePrefix {
    payload_len: u32,
    checksum_lo: u32,
    sequence: u64,
    hlc_seconds: u64,
    hlc_subseconds: u32,
    flags: u8,
    reserved: u8,
    principal_len_p1: u16,
    stored_extent: u32,
}

impl FramePrefix {
    /// Bytes from the frame start to the first payload byte.
    ///
    /// Computable from the verified prefix alone, and deliberately independent
    /// of the semantic checks in [`read_frame_extent`]: a frame that declares an
    /// over-cap payload or an undefined flag still occupies a knowable span, and
    /// the scan needs that span precisely when the frame is otherwise invalid.
    pub(crate) const fn header_len(&self) -> usize {
        FIXED_ENTRY_HEADER_BYTES + self.origin_tail_len() + self.principal_len()
    }

    /// Declared payload length, before the cap is enforced.
    pub(crate) const fn payload_len(&self) -> u32 {
        self.payload_len
    }

    const fn principal_len(&self) -> usize {
        self.principal_len_p1.saturating_sub(1) as usize
    }

    const fn origin_tail_len(&self) -> usize {
        if self.flags & FLAG_REPLICATED_ORIGIN == 0 {
            0
        } else {
            REPLICATED_TAIL_BYTES
        }
    }
}

/// Read and authenticate a frame's fixed prefix.
///
/// The checksum is verified before ANY field is read out, because two of them —
/// `payload_len` and `principal_len` — become read lengths and file offsets. A
/// corrupt length must never reach that arithmetic.
///
/// Every error from here means the prefix could not be authenticated, so the
/// frame's extent is unknown. Failures that happen *after* this point belong to
/// [`read_frame_extent`] and do have a knowable extent; the split is what keeps
/// those two cases distinguishable to the caller.
pub(crate) fn read_frame_prefix<R: Read>(
    reader: &mut R,
    offset: u64,
) -> PersistResult<FramePrefix> {
    let mut fixed = [0_u8; FIXED_ENTRY_HEADER_BYTES];
    read_exact_header(reader, &mut fixed, offset)?;

    let stored_prefix = u32::from_le_bytes(fixed[0..4].try_into().expect("prefix checksum"));
    if stored_prefix != crate::payload::checksum_lo(&fixed[PREFIX_CHECKSUM_COVERAGE]) {
        return Err(PersistError::WalHeaderChecksumMismatch { offset });
    }

    Ok(FramePrefix {
        payload_len: u32::from_le_bytes(fixed[4..8].try_into().expect("payload_len")),
        checksum_lo: u32::from_le_bytes(fixed[8..12].try_into().expect("payload checksum")),
        sequence: u64::from_le_bytes(fixed[12..20].try_into().expect("sequence")),
        hlc_seconds: u64::from_le_bytes(fixed[20..28].try_into().expect("hlc_seconds")),
        hlc_subseconds: u32::from_le_bytes(fixed[28..32].try_into().expect("hlc_subseconds")),
        flags: fixed[32],
        reserved: fixed[33],
        principal_len_p1: u16::from_le_bytes(fixed[34..36].try_into().expect("principal_len")),
        stored_extent: u32::from_le_bytes(fixed[36..40].try_into().expect("extent checksum")),
    })
}

/// Validate an authenticated prefix and read the extent it describes.
///
/// Every error from here leaves [`FramePrefix::header_len`] valid, so a caller
/// deciding whether a bad frame is the last thing in the file can still say
/// where it ends.
pub(crate) fn read_frame_extent<R: Read>(
    reader: &mut R,
    prefix: &FramePrefix,
    offset: u64,
) -> PersistResult<WalEntryHeader> {
    if prefix.flags & !KNOWN_FLAGS != 0 {
        return Err(PersistError::UnsupportedFlag {
            flag: u16::from(prefix.flags & !KNOWN_FLAGS),
        });
    }
    if prefix.reserved != 0 {
        return Err(PersistError::ReservedBytesNonZero {
            offset: offset + 33,
        });
    }
    ensure_payload_len(prefix.payload_len as usize)?;
    let principal_len = prefix.principal_len();
    if principal_len > MAX_PRINCIPAL_BYTES {
        return Err(PersistError::PrincipalTooLarge {
            len: principal_len,
            max: MAX_PRINCIPAL_BYTES,
        });
    }

    let origin_tail_len = prefix.origin_tail_len();
    let mut extent = vec![0_u8; origin_tail_len + principal_len];
    read_exact_header(reader, &mut extent, offset)?;
    if prefix.stored_extent != crate::payload::checksum_lo(&extent) {
        return Err(PersistError::WalHeaderChecksumMismatch { offset });
    }

    let origin = if origin_tail_len == 0 {
        Origin::Local
    } else {
        let source_node_id = u64::from_le_bytes(extent[0..8].try_into().expect("source_node_id"));
        let source_seq = u64::from_le_bytes(extent[8..16].try_into().expect("source_seq"));
        Origin::Replicated {
            source_node_id: NodeId::new(source_node_id),
            source_seq,
        }
    };
    let principal = if prefix.principal_len_p1 == 0 {
        None
    } else {
        Some(Arc::from(&extent[origin_tail_len..]))
    };

    Ok(WalEntryHeader {
        payload_len: prefix.payload_len,
        checksum_lo: prefix.checksum_lo,
        sequence: prefix.sequence,
        hlc_seconds: prefix.hlc_seconds,
        hlc_subseconds: prefix.hlc_subseconds,
        origin,
        flags: prefix.flags & !FLAG_REPLICATED_ORIGIN,
        principal,
    })
}

pub(crate) fn read_entry_header<R: Read>(
    reader: &mut R,
    offset: u64,
) -> PersistResult<(WalEntryHeader, usize)> {
    let prefix = read_frame_prefix(reader, offset)?;
    let header_len = prefix.header_len();
    let header = read_frame_extent(reader, &prefix, offset)?;
    Ok((header, header_len))
}

fn read_exact_header(reader: &mut impl Read, buf: &mut [u8], offset: u64) -> PersistResult<()> {
    reader.read_exact(buf).map_err(|error| match error.kind() {
        ErrorKind::UnexpectedEof => PersistError::TruncatedEntry { offset },
        _ => PersistError::Io(error),
    })
}

#[cfg(test)]
#[path = "entry_header/tests.rs"]
mod tests;
