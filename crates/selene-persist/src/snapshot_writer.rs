//! Atomic snapshot envelope writer.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use crate::compression::compress_zstd;
use crate::section::{
    MAX_SECTION_COUNT, SECTION_TABLE_ROW_LEN, SectionEntry, body_hash, section_table_bytes,
    validate_section_payload_len,
};
use crate::snapshot_file_header::{
    FLAG_SECTION_COMPRESSED, SNAPSHOT_FILE_HEADER_LEN, SnapshotFileHeader,
};
use crate::snapshot_path::{snapshot_path, snapshot_tmp_path};
use crate::{PersistError, PersistResult};

/// Snapshot section compression mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SectionCompression {
    /// Store section payloads uncompressed.
    None,
    /// Compress each section independently using zstd.
    PerSection {
        /// zstd compression level.
        level: i32,
    },
}

impl SectionCompression {
    /// Default v1.0 compression mode.
    pub const DEFAULT: Self = Self::PerSection { level: 1 };
}

impl Default for SectionCompression {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Snapshot write configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotConfig {
    /// Directory where `snapshot.{sequence}.snap` is written.
    pub dir: PathBuf,
    /// Snapshot sequence.
    pub sequence: u64,
    /// Section compression mode.
    pub compression: SectionCompression,
    /// Whether to fsync the snapshot file before rename.
    pub fsync: bool,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            dir: PathBuf::new(),
            sequence: 0,
            compression: SectionCompression::DEFAULT,
            fsync: true,
        }
    }
}

/// Builder for one snapshot envelope.
#[derive(Debug)]
pub struct SnapshotBuilder {
    config: SnapshotConfig,
    sections: Vec<RawSection>,
    seen: HashSet<([u8; 4], [u8; 4])>,
}

#[derive(Clone, Debug)]
struct RawSection {
    provider: [u8; 4],
    sub: [u8; 4],
    payload: Vec<u8>,
}

struct PreparedSection {
    entry: SectionEntry,
    payload: Vec<u8>,
}

impl SnapshotBuilder {
    /// Construct an empty snapshot builder.
    #[must_use]
    pub fn new(config: SnapshotConfig) -> Self {
        Self {
            config,
            sections: Vec::new(),
            seen: HashSet::new(),
        }
    }

    /// Add one opaque section payload.
    ///
    /// # Errors
    ///
    /// Returns cap errors for too many sections or oversized payloads, and
    /// [`PersistError::DuplicateSection`] if the tag pair was already added.
    pub fn add_section(
        &mut self,
        provider: [u8; 4],
        sub: [u8; 4],
        payload: Vec<u8>,
    ) -> PersistResult<()> {
        if self.sections.len() == MAX_SECTION_COUNT {
            return Err(PersistError::TooManySections {
                count: self.sections.len() + 1,
                max: MAX_SECTION_COUNT,
            });
        }
        validate_section_payload_len(payload.len())?;
        if !self.seen.insert((provider, sub)) {
            return Err(PersistError::DuplicateSection { provider, sub });
        }
        self.sections.push(RawSection {
            provider,
            sub,
            payload,
        });
        Ok(())
    }

    /// Atomically write the snapshot and return the final path.
    ///
    /// The writer creates `snapshot.{sequence}.snap.tmp`, writes and optionally
    /// fsyncs it, then renames it to `snapshot.{sequence}.snap`. The parent
    /// directory is not fsynced in v1.0.
    ///
    /// # Errors
    ///
    /// Returns cap, compression, hash/header construction, or I/O errors.
    pub fn finalize(self) -> PersistResult<PathBuf> {
        let final_path = snapshot_path(&self.config.dir, self.config.sequence);
        if final_path.try_exists()? {
            return Err(PersistError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "snapshot already exists for sequence",
            )));
        }
        let tmp_path = snapshot_tmp_path(&self.config.dir, self.config.sequence);
        let prepared = prepare_sections(&self.sections, self.config.compression)?;
        let entries: Vec<_> = prepared.iter().map(|section| section.entry).collect();
        let table = section_table_bytes(&entries)?;
        let hash = body_hash(
            &table,
            prepared.iter().map(|section| section.payload.as_slice()),
        );
        let flags = match self.config.compression {
            SectionCompression::None => 0,
            SectionCompression::PerSection { .. } => FLAG_SECTION_COMPRESSED,
        };
        let header = SnapshotFileHeader::new(flags, entries.len(), hash)?;
        let mut writer = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?,
        );
        header.write_to(&mut writer)?;
        writer.write_all(&table)?;
        for section in prepared {
            writer.write_all(&section.payload)?;
        }
        writer.flush()?;
        if self.config.fsync {
            writer.get_ref().sync_data()?;
        }
        drop(writer);
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(final_path)
    }
}

fn prepare_sections(
    sections: &[RawSection],
    compression: SectionCompression,
) -> PersistResult<Vec<PreparedSection>> {
    let mut payload_offset =
        SNAPSHOT_FILE_HEADER_LEN as u64 + (sections.len() * SECTION_TABLE_ROW_LEN) as u64;
    let mut prepared = Vec::with_capacity(sections.len());
    for section in sections {
        let payload = match compression {
            SectionCompression::None => section.payload.clone(),
            SectionCompression::PerSection { level } => compress_zstd(&section.payload, level)?,
        };
        validate_section_payload_len(payload.len())?;
        let payload_len = payload.len() as u64;
        prepared.push(PreparedSection {
            entry: SectionEntry {
                provider: section.provider,
                sub: section.sub,
                payload_offset,
                payload_len,
            },
            payload,
        });
        payload_offset = payload_offset.saturating_add(payload_len);
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::{SnapshotReader, snapshot_path, snapshot_tmp_path};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "selene-snapshot-writer-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        dir
    }

    fn config(dir: PathBuf, sequence: u64, compression: SectionCompression) -> SnapshotConfig {
        SnapshotConfig {
            dir,
            sequence,
            compression,
            fsync: true,
        }
    }

    #[test]
    fn empty_snapshot_round_trips() {
        let dir = temp_dir("empty");
        let path = SnapshotBuilder::new(config(dir.clone(), 1, SectionCompression::None))
            .finalize()
            .unwrap();
        let mut reader = SnapshotReader::open(&path).unwrap();
        reader.verify_body_hash().unwrap();
        assert!(reader.sections().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn single_section_round_trips_per_section_compressed() {
        let dir = temp_dir("compressed");
        let mut builder = SnapshotBuilder::new(config(
            dir.clone(),
            2,
            SectionCompression::PerSection { level: 1 },
        ));
        builder
            .add_section(*b"CORE", *b"META", vec![7_u8; 1024])
            .unwrap();
        let path = builder.finalize().unwrap();
        let mut reader = SnapshotReader::open(&path).unwrap();
        assert!(reader.header().is_section_compressed());
        assert_eq!(
            reader.read_section(*b"CORE", *b"META").unwrap(),
            vec![7_u8; 1024]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn single_section_round_trips_uncompressed() {
        let dir = temp_dir("raw");
        let mut builder = SnapshotBuilder::new(config(dir.clone(), 3, SectionCompression::None));
        builder
            .add_section(*b"CORE", *b"NODE", b"nodes".to_vec())
            .unwrap();
        let path = builder.finalize().unwrap();
        let mut reader = SnapshotReader::open(&path).unwrap();
        assert!(!reader.header().is_section_compressed());
        assert_eq!(reader.read_section(*b"CORE", *b"NODE").unwrap(), b"nodes");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_provider_sub_is_rejected() {
        let dir = temp_dir("dup");
        let mut builder = SnapshotBuilder::new(config(dir.clone(), 4, SectionCompression::None));
        builder.add_section(*b"CORE", *b"META", vec![]).unwrap();
        assert!(matches!(
            builder.add_section(*b"CORE", *b"META", vec![]),
            Err(PersistError::DuplicateSection { provider, sub })
                if provider == *b"CORE" && sub == *b"META"
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn section_size_boundary_uses_validator_without_allocating() {
        crate::section::validate_section_payload_len(crate::MAX_SECTION_PAYLOAD_BYTES).unwrap();
        assert!(matches!(
            crate::section::validate_section_payload_len(crate::MAX_SECTION_PAYLOAD_BYTES + 1),
            Err(PersistError::SectionTooLarge { .. })
        ));
    }

    #[test]
    fn stale_tmp_prevents_write_and_leaves_no_final() {
        let dir = temp_dir("stale-tmp");
        fs::write(snapshot_tmp_path(&dir, 5), b"partial").unwrap();
        let err = SnapshotBuilder::new(config(dir.clone(), 5, SectionCompression::None))
            .finalize()
            .unwrap_err();
        assert!(matches!(err, PersistError::Io(_)));
        assert!(snapshot_tmp_path(&dir, 5).exists());
        assert!(!snapshot_path(&dir, 5).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn final_snapshot_path_already_exists_is_rejected_before_rename() {
        let dir = temp_dir("final-exists");
        fs::write(snapshot_path(&dir, 6), b"existing").unwrap();
        let err = SnapshotBuilder::new(config(dir.clone(), 6, SectionCompression::None))
            .finalize()
            .unwrap_err();
        assert!(matches!(
            err,
            PersistError::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert!(!snapshot_tmp_path(&dir, 6).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn byte_identical_uncompressed_writes() {
        let dir = temp_dir("identical");
        for sequence in [7, 8] {
            let mut builder =
                SnapshotBuilder::new(config(dir.clone(), sequence, SectionCompression::None));
            builder
                .add_section(*b"CORE", *b"META", b"meta".to_vec())
                .unwrap();
            builder
                .add_section(*b"CORE", *b"NODE", b"nodes".to_vec())
                .unwrap();
            builder.finalize().unwrap();
        }
        let left = fs::read(snapshot_path(&dir, 7)).unwrap();
        let right = fs::read(snapshot_path(&dir, 8)).unwrap();
        assert_eq!(left, right);
        let _ = fs::remove_dir_all(dir);
    }
}
