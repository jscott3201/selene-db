//! WAL append paths, including typed checkpoint watermarks.

use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;

use selene_core::{Change, HlcTimestamp, Origin, metrics};

use super::{SyncPolicy, WalWriter};
use crate::entry_header::{FLAG_CHECKPOINT_WATERMARK, encode_entry_header, validate_principal};
use crate::payload::encode_changes_with_compressor;
use crate::{PersistError, PersistResult, WalEntryHeader};

pub(super) const WAL_RECORD_BUFFER_RETAIN_LIMIT: usize = 4 * 1024 * 1024;

impl WalWriter {
    /// Append one logical WAL commit and return its assigned sequence.
    ///
    /// # Errors
    ///
    /// Returns [`PersistError::WalWriterPoisoned`] after rotation detects
    /// persistence-state divergence, or sequence, codec, cap, compression, or
    /// I/O errors. On error, the sequence counter is not advanced and the file
    /// is rolled back to the last fully committed entry when possible.
    #[tracing::instrument(
        name = "selene.persist.wal.append",
        skip(self, principal, changes),
        fields(sequence = self.last_sequence.saturating_add(1), change_count = changes.len(), has_principal = principal.is_some())
    )]
    pub fn append(
        &mut self,
        hlc: HlcTimestamp,
        origin: Origin,
        principal: Option<Arc<[u8]>>,
        changes: &[Change],
    ) -> PersistResult<u64> {
        self.append_record(hlc, origin, principal, changes, 0, false)
    }

    /// Append one typed physical checkpoint watermark without flushing it.
    ///
    /// The enclosing checkpoint rotation supplies the durability barrier. This
    /// record advances physical WAL sequence only: it is local, principal-free,
    /// and carries an empty change payload.
    pub(super) fn append_checkpoint_watermark_record(
        &mut self,
        hlc: HlcTimestamp,
    ) -> PersistResult<u64> {
        self.append_record(
            hlc,
            Origin::Local,
            None,
            &[],
            FLAG_CHECKPOINT_WATERMARK,
            true,
        )
    }

    fn append_record(
        &mut self,
        hlc: HlcTimestamp,
        origin: Origin,
        principal: Option<Arc<[u8]>>,
        changes: &[Change],
        extra_flags: u8,
        defer_policy_fsync: bool,
    ) -> PersistResult<u64> {
        self.ensure_usable()?;
        validate_principal(principal.as_deref())?;
        let payload =
            encode_changes_with_compressor(changes, self.compression, Some(&mut self.compressor))?;
        let sequence =
            self.last_sequence
                .checked_add(1)
                .ok_or(PersistError::WalSequenceExhausted {
                    last_sequence: self.last_sequence,
                })?;
        let header = WalEntryHeader::new(
            payload.bytes.len(),
            payload.checksum_lo,
            sequence,
            hlc,
            origin,
            payload.flags | extra_flags,
            principal,
        )?;
        let header_bytes = encode_entry_header(&header)?;
        let pending_count = self.entries_since_fsync.saturating_add(1);
        let needs_fsync = !defer_policy_fsync
            && match self.sync_policy {
                SyncPolicy::EveryN(threshold) => pending_count >= threshold,
                SyncPolicy::OnFlushOnly => false,
            };

        self.record.clear();
        self.record
            .reserve(header_bytes.len() + payload.bytes.len());
        self.record.extend_from_slice(&header_bytes);
        self.record.extend_from_slice(&payload.bytes);
        let record_len = self.record.len();

        let result = (|| -> PersistResult<()> {
            self.file.write_all(&self.record)?;
            if needs_fsync {
                self.file.sync_data()?;
            }
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.committed_offset = self.committed_offset.saturating_add(record_len as u64);
                self.last_sequence = sequence;
                self.entries_since_fsync = if needs_fsync { 0 } else { pending_count };
                metrics::counter_inc(metrics::WAL_APPENDS_TOTAL);
                self.trim_record_buffer();
                Ok(sequence)
            }
            Err(error) => {
                self.rollback_to_committed_offset();
                self.trim_record_buffer();
                Err(error)
            }
        }
    }

    fn trim_record_buffer(&mut self) {
        if self.record.capacity() > WAL_RECORD_BUFFER_RETAIN_LIMIT {
            self.record = Vec::new();
        }
    }

    /// Best-effort rollback to the last committed offset on append failure.
    fn rollback_to_committed_offset(&mut self) {
        if let Err(error) = self.file.set_len(self.committed_offset) {
            tracing::error!(%error, "failed to truncate WAL after append error");
            return;
        }
        if let Err(error) = self.file.seek(SeekFrom::Start(self.committed_offset)) {
            tracing::error!(%error, "failed to seek WAL after append error");
        }
    }
}
