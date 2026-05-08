//! Append-only WAL writer.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use selene_core::{Change, HlcTimestamp, Origin};

use crate::entry_header::{
    encode_entry_header, ensure_payload_len, read_entry_header, validate_principal,
};
use crate::file_header::{WAL_FILE_HEADER_LEN, WalFileHeader};
use crate::payload::{encode_changes, verify_checksum};
use crate::{PersistError, PersistResult, WalEntryHeader};

/// Conventional v1.0 single-file WAL name used by embedders.
pub const DEFAULT_WAL_FILE_NAME: &str = "wal.log";

/// WAL writer configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WalConfig {
    /// Flush and fsync after this many appended entries.
    ///
    /// `1` is durability-by-default. Values greater than `1` opt into group
    /// commit. `0` is normalized to `1`.
    pub fsync_every_n: u32,
    /// Highest WAL sequence covered by the snapshot this file extends.
    pub snapshot_seq: u64,
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            fsync_every_n: 1,
            snapshot_seq: 0,
        }
    }
}

/// Single-threaded append-only WAL writer.
pub struct WalWriter {
    writer: BufWriter<File>,
    last_sequence: u64,
    fsync_every_n: u32,
    entries_since_fsync: u32,
}

impl WalWriter {
    /// Open a WAL file for append, creating the v1 header for a new file.
    ///
    /// Existing files are scanned once to find the last valid entry. A partial
    /// or checksum-invalid tail is truncated to the last valid offset.
    ///
    /// # Errors
    ///
    /// Returns I/O, header, sequence, or checksum errors encountered while
    /// opening and validating the WAL.
    pub fn open(path: &Path, config: WalConfig) -> PersistResult<Self> {
        let fsync_every_n = config.fsync_every_n.max(1);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        let len = file.metadata()?.len();
        if len == 0 {
            WalFileHeader::new(config.snapshot_seq).write_to(&mut file)?;
            file.sync_data()?;
        } else {
            file.seek(SeekFrom::Start(0))?;
            WalFileHeader::read_from(&mut file)?;
        }

        let scan = scan_existing(&mut file)?;
        if scan.truncate_to < file.metadata()?.len() {
            tracing::warn!(
                offset = scan.truncate_to,
                "truncating WAL tail to last valid entry"
            );
            file.set_len(scan.truncate_to)?;
        }
        file.seek(SeekFrom::Start(scan.truncate_to))?;

        Ok(Self {
            writer: BufWriter::new(file),
            last_sequence: scan.last_sequence,
            fsync_every_n,
            entries_since_fsync: 0,
        })
    }

    /// Append one WAL entry and return its assigned sequence.
    ///
    /// # Errors
    ///
    /// Returns codec, cap, compression, or I/O errors. On error, the in-memory
    /// sequence counter is not advanced.
    pub fn append(
        &mut self,
        hlc: HlcTimestamp,
        origin: Origin,
        principal: Option<Arc<[u8]>>,
        changes: &[Change],
    ) -> PersistResult<u64> {
        validate_principal(principal.as_deref())?;
        let payload = encode_changes(changes)?;
        let sequence = self.last_sequence + 1;
        let header = WalEntryHeader::new(
            payload.bytes.len(),
            payload.checksum_lo,
            sequence,
            hlc,
            origin,
            payload.flags,
            principal,
        )?;
        let header_bytes = encode_entry_header(&header)?;
        self.writer.write_all(&header_bytes)?;
        self.writer.write_all(&payload.bytes)?;
        self.last_sequence = sequence;
        self.entries_since_fsync += 1;
        if self.entries_since_fsync >= self.fsync_every_n {
            self.flush()?;
        }
        Ok(sequence)
    }

    /// Flush buffered bytes and fsync the WAL file.
    ///
    /// # Errors
    ///
    /// Returns I/O errors from flush or fsync.
    pub fn flush(&mut self) -> PersistResult<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_data()?;
        self.entries_since_fsync = 0;
        Ok(())
    }

    /// Return the last sequence assigned by this writer.
    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }
}

impl Drop for WalWriter {
    fn drop(&mut self) {
        if let Err(error) = self.flush() {
            tracing::error!(%error, "failed to flush WAL writer on drop");
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Scan {
    last_sequence: u64,
    truncate_to: u64,
}

fn scan_existing(file: &mut File) -> PersistResult<Scan> {
    let file_len = file.metadata()?.len();
    let mut offset = WAL_FILE_HEADER_LEN as u64;
    let mut previous = 0_u64;
    let mut last_valid_offset = offset;

    loop {
        if offset == file_len {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }
        if offset > file_len {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }

        file.seek(SeekFrom::Start(offset))?;
        let header = match read_entry_header(&mut *file, offset) {
            Ok((header, _)) => header,
            Err(PersistError::TruncatedEntry { .. }) | Err(PersistError::HeaderCodec(_)) => {
                return Ok(Scan {
                    last_sequence: previous,
                    truncate_to: last_valid_offset,
                });
            }
            Err(error) => return Err(error),
        };

        if header.sequence <= previous {
            return Err(PersistError::NonMonotonicSequence {
                previous,
                current: header.sequence,
            });
        }
        let payload_len = header.payload_len as usize;
        if ensure_payload_len(payload_len).is_err() {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }
        let payload_start = file.stream_position()?;
        let payload_end = payload_start.saturating_add(u64::from(header.payload_len));
        if payload_end > file_len {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }
        let mut payload = vec![0_u8; payload_len];
        file.read_exact(&mut payload)?;
        if verify_checksum(&header, &payload).is_err() {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }
        previous = header.sequence;
        offset = payload_end;
        last_valid_offset = payload_end;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use selene_core::{Change, NodeId, Origin, PropertyMap, intern};

    use super::*;
    use crate::{MAX_PRINCIPAL_BYTES, WAL_FILE_HEADER_LEN, WalReader};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "selene-persist-{name}-{}-{nanos}.wal",
            std::process::id()
        ))
    }

    fn changes() -> Vec<Change> {
        vec![Change::NodeCreated {
            id: NodeId::new(1),
            labels: selene_core::LabelSet::single(intern("writer.node").unwrap()),
            properties: PropertyMap::new(),
        }]
    }

    #[test]
    fn open_new_file_writes_header() {
        let path = temp_path("open-new");
        {
            let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
            assert_eq!(writer.last_sequence(), 0);
        }
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            WAL_FILE_HEADER_LEN as u64
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn append_assigns_monotonic_sequences() {
        let path = temp_path("seq");
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        assert_eq!(
            writer
                .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
                .unwrap(),
            1
        );
        assert_eq!(
            writer
                .append(HlcTimestamp::new(1, 1), Origin::Local, None, &changes())
                .unwrap(),
            2
        );
        assert_eq!(writer.last_sequence(), 2);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reopen_recovers_last_sequence() {
        let path = temp_path("reopen");
        {
            let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
            writer
                .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
                .unwrap();
        }
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        assert_eq!(writer.last_sequence(), 1);
        assert_eq!(
            writer
                .append(HlcTimestamp::new(2, 0), Origin::Local, None, &changes())
                .unwrap(),
            2
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn principal_overflow_does_not_increment_sequence() {
        let path = temp_path("principal");
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        let len_before = fs::metadata(&path).unwrap().len();
        let err = writer
            .append(
                HlcTimestamp::zero(),
                Origin::Local,
                Some(Arc::from(vec![1_u8; MAX_PRINCIPAL_BYTES + 1])),
                &changes(),
            )
            .unwrap_err();
        assert!(matches!(err, PersistError::PrincipalTooLarge { .. }));
        assert_eq!(writer.last_sequence(), 0);
        assert_eq!(fs::metadata(&path).unwrap().len(), len_before);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn partial_tail_is_truncated_on_open() {
        let path = temp_path("tail");
        {
            let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
            writer
                .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
                .unwrap();
            writer.flush().unwrap();
        }
        let valid_len = fs::metadata(&path).unwrap().len();
        {
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&[0, 1, 2]).unwrap();
        }
        let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        assert_eq!(writer.last_sequence(), 1);
        assert_eq!(fs::metadata(&path).unwrap().len(), valid_len);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn group_commit_defers_fsync_until_threshold() {
        let path = temp_path("group");
        let mut writer = WalWriter::open(
            &path,
            WalConfig {
                fsync_every_n: 3,
                snapshot_seq: 0,
            },
        )
        .unwrap();
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap();
        writer
            .append(HlcTimestamp::new(1, 1), Origin::Local, None, &changes())
            .unwrap();
        assert_eq!(writer.entries_since_fsync, 2);
        writer
            .append(HlcTimestamp::new(1, 2), Origin::Local, None, &changes())
            .unwrap();
        assert_eq!(writer.entries_since_fsync, 0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn replicated_origin_node_id_is_written() {
        let path = temp_path("origin");
        {
            let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
            writer
                .append(
                    HlcTimestamp::new(1, 0),
                    Origin::Replicated {
                        source_node_id: NodeId::new(77),
                        source_seq: 9,
                    },
                    None,
                    &changes(),
                )
                .unwrap();
        }
        let reader = WalReader::open(&path).unwrap();
        let entry = reader.iterate(|_| true).unwrap().next().unwrap().unwrap();
        assert_eq!(entry.header.origin_node_id, 77);
        let _ = fs::remove_file(path);
    }
}
