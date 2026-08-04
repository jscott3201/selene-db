//! WAL tail scanning, and the rule that separates a torn tail from corruption.
//!
//! Opening a WAL means deciding what to do with bytes that do not parse. Two
//! very different things produce them:
//!
//! - A **torn tail**: the process died mid-append, or the filesystem extended
//!   the file and lost the data before the metadata. The bytes were never
//!   fsynced, so no commit was ever acknowledged for them, and discarding them
//!   is exactly right — it is ordinary crash recovery.
//! - **Corruption**: bit rot, a lost page, a bad cable, under a prefix that was
//!   fsynced and acknowledged. Discarding those bytes destroys committed data,
//!   and destroys the evidence needed to recover it by other means.
//!
//! Refusing preserves the bytes; it does not by itself recover them. There is
//! no repair tool in this crate yet, and `WalEntryStream` stops at the first
//! error rather than skipping a bad frame, so today the guarantee is exactly
//! "the file is still there, unmodified" — which is the precondition for any
//! recovery, and strictly more than truncation leaves.
//!
//! Before the v3 frame layout the two were indistinguishable — both surfaced as
//! a checksum mismatch or an implausible length — and both were handled by
//! truncating everything from the failure to end of file. A corrupt frame in
//! the middle of a log silently took every committed frame after it.
//!
//! The v3 prefix checksum is what makes the rule possible: a frame's declared
//! extent can be trusted, so "does this frame run past the end of the file?" is
//! a question about the frame rather than about an unprotected integer.
//!
//! # The rule
//!
//! The discriminator is **does anything follow the failing frame?** The writer
//! only ever appends, so bytes beyond a frame's extent prove that frame was
//! fully written before them — it cannot have been torn. Nothing beyond it
//! means it is the last thing in the file, and discarding one frame that was
//! never acknowledged is ordinary crash recovery.
//!
//! For a frame at `offset` that fails validation:
//!
//! 1. **Prefix checksum passed, so the extent is trustworthy.** If the frame
//!    ends at or past end of file, nothing follows it: torn. If it ends before
//!    end of file, committed bytes follow it: corruption. This covers every
//!    failure after the prefix — a bad extent checksum, an undefined flag, an
//!    over-cap length — not only a bad payload checksum. A frame whose
//!    principal rotted is no less a tail than one whose payload did.
//! 2. **Prefix checksum failed, so there is no trustworthy extent.** Fall back
//!    to shape: if everything from `offset` to end of file is zero — what a
//!    short extending write leaves, and what no valid frame can be — torn.
//!    Otherwise corruption.
//!
//! 3. **Corruption refuses**: return [`PersistError::WalMidLogCorruption`]
//!    wrapping the underlying failure, and leave the file untouched so the
//!    committed frames survive. The wrapper is the
//!    part that matters to an operator: a bare checksum error does not say
//!    whether the log survived, and that ambiguity is what let the old
//!    truncate-and-continue behaviour pass for recovery.
//!
//! The split between 1 and 2 is enforced by types rather than by care:
//! [`read_frame_prefix`] returns a `FramePrefix` only once the checksum passes,
//! and everything that can fail with a knowable extent lives behind
//! [`read_frame_extent`], which needs one. Routing a stage-2 failure into the
//! stage-1 fallback is exactly the bug this shape exists to prevent.
//!
//! # What this cannot decide
//!
//! No rule read from the file alone can separate "never reached disk" from
//! "reached disk and then rotted", because the bytes are identical and differ
//! only in when they became durable — which the file does not record. A lone
//! trailing frame that rotted to zeros is still treated as a tear. The bound
//! that matters holds regardless: nothing before the tail is ever discarded.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

use crate::entry_header::{read_frame_extent, read_frame_prefix};
use crate::file_header::WAL_FILE_HEADER_LEN;
use crate::payload::verify_checksum;
use crate::{PersistError, PersistResult};

/// Why a WAL's trailing bytes were discarded when it was opened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WalTailReason {
    /// The file ended part-way through the final frame.
    ShortFrame,
    /// The final frame was complete on disk but failed validation, and nothing
    /// followed it. A crash can leave a fully extended but partly stale frame;
    /// it was never acknowledged, so discarding it is correct.
    CorruptFinalFrame,
    /// The trailing bytes were entirely zero, as a short extending write leaves.
    ZeroFilledTail,
}

/// A repair performed while opening a WAL.
///
/// Reported rather than merely logged: a `tracing::warn!` is invisible to an
/// embedder with no subscriber, and losing an unacknowledged tail is still
/// something an operator may want to know happened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WalTailRepair {
    /// Why the tail was discarded.
    pub reason: WalTailReason,
    /// File offset where the discarded region began.
    pub offset: u64,
    /// Number of bytes discarded.
    pub discarded_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Scan {
    pub(crate) last_sequence: u64,
    pub(crate) truncate_to: u64,
    pub(crate) repair: Option<WalTailRepair>,
}

/// Buffer used for the zero-tail probe. Fixed, never sized from file content.
const ZERO_PROBE_CHUNK: usize = 64 * 1024;

impl Scan {
    fn intact(last_sequence: u64, truncate_to: u64) -> Self {
        Self {
            last_sequence,
            truncate_to,
            repair: None,
        }
    }
}

/// Scan an open WAL, returning where the valid prefix ends.
///
/// Returns `Err` — without proposing any truncation — when the failure cannot
/// be a torn tail. The caller must not modify the file in that case; that is
/// the whole point of the classification.
pub(crate) fn scan_existing(file: &mut File) -> PersistResult<Scan> {
    let file_len = file.metadata()?.len();
    let mut offset = WAL_FILE_HEADER_LEN as u64;
    let mut previous = 0_u64;
    let mut payload = Vec::new();
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::with_capacity(ZERO_PROBE_CHUNK, file);

    loop {
        if offset >= file_len {
            return Ok(Scan::intact(previous, offset.min(file_len)));
        }
        let frame_start = offset;

        // Stage 1: authenticate the prefix. Failing here means the frame's
        // extent is unknown, so the fallback probe is the only tool left.
        let prefix = match read_frame_prefix(&mut reader, frame_start) {
            Ok(prefix) => prefix,
            // A short read means the file ended inside this frame; nothing can
            // follow it, so it is a tear by construction and needs no probe.
            Err(PersistError::TruncatedEntry { .. }) => {
                return torn(previous, frame_start, file_len, WalTailReason::ShortFrame);
            }
            Err(error) => {
                return classify_untrusted(&mut reader, previous, frame_start, file_len, error);
            }
        };

        // From here the extent IS trustworthy: header_len and payload_len both
        // come from bytes the prefix checksum covered. Every later failure —
        // a bad extent checksum, an undefined flag, an over-cap length — is
        // therefore classifiable by where the frame ends, not by a zero probe.
        let payload_start = frame_start.saturating_add(prefix.header_len() as u64);
        let payload_end = payload_start.saturating_add(u64::from(prefix.payload_len()));
        if payload_end > file_len {
            return torn(previous, frame_start, file_len, WalTailReason::ShortFrame);
        }

        // Stage 2: validate and read what the prefix described.
        let header = match read_frame_extent(&mut reader, &prefix, frame_start) {
            Ok(header) => header,
            Err(error) => {
                return classify_by_extent(previous, frame_start, payload_end, file_len, error);
            }
        };

        let payload_len = header.payload_len as usize;
        if header.sequence <= previous {
            return classify_by_extent(
                previous,
                frame_start,
                payload_end,
                file_len,
                PersistError::NonMonotonicSequence {
                    previous,
                    current: header.sequence,
                },
            );
        }
        payload.resize(payload_len, 0);
        reader.read_exact(&mut payload)?;
        if let Err(error) = verify_checksum(&header, &payload) {
            return classify_by_extent(previous, frame_start, payload_end, file_len, error);
        }
        previous = header.sequence;
        offset = payload_end;
    }
}

fn torn(
    last_sequence: u64,
    frame_start: u64,
    file_len: u64,
    reason: WalTailReason,
) -> PersistResult<Scan> {
    Ok(Scan {
        last_sequence,
        truncate_to: frame_start,
        repair: Some(WalTailRepair {
            reason,
            offset: frame_start,
            discarded_bytes: file_len.saturating_sub(frame_start),
        }),
    })
}

/// Classify a failure whose frame extent IS trustworthy.
///
/// The frame's prefix checksum passed, so `payload_end` is where the writer
/// would have continued. Bytes beyond it were appended after this frame was
/// complete, which makes this frame corrupt rather than torn.
fn classify_by_extent(
    last_sequence: u64,
    frame_start: u64,
    payload_end: u64,
    file_len: u64,
    error: PersistError,
) -> PersistResult<Scan> {
    if payload_end >= file_len {
        return torn(
            last_sequence,
            frame_start,
            file_len,
            WalTailReason::CorruptFinalFrame,
        );
    }
    Err(refuse(frame_start, file_len, error))
}

/// Classify a failure whose frame extent is NOT trustworthy.
///
/// The prefix checksum failed, so nothing in the frame can be believed and
/// there is no extent to reason about. Fall back to shape: a short extending
/// write leaves zeros, and no valid frame is all-zero.
fn classify_untrusted(
    reader: &mut BufReader<&mut File>,
    last_sequence: u64,
    frame_start: u64,
    file_len: u64,
    error: PersistError,
) -> PersistResult<Scan> {
    if tail_is_zero(reader, frame_start, file_len)? {
        return torn(
            last_sequence,
            frame_start,
            file_len,
            WalTailReason::ZeroFilledTail,
        );
    }
    Err(refuse(frame_start, file_len, error))
}

/// Wrap a validation failure as a refusal, so the caller can tell from the
/// error alone that the file was left intact.
fn refuse(frame_start: u64, file_len: u64, source: PersistError) -> PersistError {
    // An I/O failure is not a verdict about the log's contents; reporting it as
    // corruption would blame the data for a failure of the device.
    if matches!(source, PersistError::Io(_)) {
        return source;
    }
    PersistError::WalMidLogCorruption {
        offset: frame_start,
        trailing_bytes: file_len.saturating_sub(frame_start),
        source: Box::new(source),
    }
}

/// Whether every byte from `from` to end of file is zero.
fn tail_is_zero(
    reader: &mut BufReader<&mut File>,
    from: u64,
    file_len: u64,
) -> PersistResult<bool> {
    reader.seek(SeekFrom::Start(from))?;
    let mut remaining = file_len.saturating_sub(from);
    let mut buffer = vec![0_u8; ZERO_PROBE_CHUNK];
    while remaining > 0 {
        let want = usize::try_from(remaining.min(ZERO_PROBE_CHUNK as u64))
            .expect("chunk length fits in usize");
        reader.read_exact(&mut buffer[..want])?;
        if buffer[..want].iter().any(|byte| *byte != 0) {
            return Ok(false);
        }
        remaining -= want as u64;
    }
    Ok(true)
}

#[cfg(test)]
#[path = "wal_tail/tests.rs"]
mod tests;
