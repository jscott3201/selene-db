//! Atomic snapshot envelope writer.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use rayon::prelude::*;
use selene_core::metrics;

use crate::compression::compress_zstd;
use crate::manifest::sync_dir;
use crate::section::{
    MAX_SECTION_COUNT, SECTION_TABLE_ROW_LEN, SectionEntry, body_hash, section_table_bytes,
    validate_section_payload_len,
};
use crate::snapshot_file_header::{
    FLAG_SECTION_COMPRESSED, SNAPSHOT_FILE_HEADER_LEN, SnapshotFileHeader,
};
use crate::snapshot_path::{snapshot_path, snapshot_tmp_path};
use crate::{PersistError, PersistResult};

const PARALLEL_SNAPSHOT_COMPRESSION_MIN_BYTES: usize = 1024 * 1024;

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

#[derive(Debug)]
struct RawSection {
    provider: [u8; 4],
    sub: [u8; 4],
    payload: Vec<u8>,
}

struct PreparedSection {
    entry: SectionEntry,
    payload: Vec<u8>,
}

/// Metadata returned after a snapshot is durably finalized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotFinalizeOutcome {
    /// Snapshot sequence written into the finalized snapshot filename.
    pub snapshot_seq: u64,
    /// Low 128 bits of the finalized snapshot body hash.
    pub body_hash: [u8; 16],
    /// Number of section-table rows in the snapshot.
    pub section_count: u32,
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

    /// Return the snapshot sequence this builder will finalize.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.config.sequence
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

    /// Atomically write the snapshot and return the finalized metadata.
    ///
    /// The writer creates `snapshot.{sequence}.snap.tmp`, writes and optionally
    /// fsyncs it, then **hard-links** it to `snapshot.{sequence}.snap` and
    /// removes the tmp. `hard_link` fails atomically with `AlreadyExists` when
    /// the final path already has a snapshot for this sequence; this is the
    /// race-safe alternative to `rename` (which silently overwrites on POSIX)
    /// without requiring `unsafe` for `renameat2(RENAME_NOREPLACE)`. On a
    /// collision the tmp is removed best-effort. When `config.fsync` is set the
    /// parent directory is fsynced after the `hard_link` so the new directory
    /// entry is durable — closing the crash window for a downstream embedder that
    /// publishes a snapshot via `finalize` directly. (The rotation path adds its
    /// own fsync-independent re-open/`sync_all` barrier on top of this.)
    ///
    /// # Errors
    ///
    /// Returns cap, compression, hash/header construction, or I/O errors,
    /// including `Io(AlreadyExists)` when the final snapshot path is taken.
    #[tracing::instrument(
        name = "selene.persist.snapshot.finalize",
        skip(self),
        fields(snapshot_seq = self.config.sequence, section_count = self.sections.len())
    )]
    pub fn finalize(self) -> PersistResult<SnapshotFinalizeOutcome> {
        let started = Instant::now();
        let final_path = snapshot_path(&self.config.dir, self.config.sequence);
        let tmp_path = snapshot_tmp_path(&self.config.dir, self.config.sequence);
        let prepared = prepare_sections(self.sections, self.config.compression)?;
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
        match std::fs::hard_link(&tmp_path, &final_path) {
            Ok(()) => {
                let _ = std::fs::remove_file(&tmp_path);
                // Make the new directory entry durable AFTER the publish (the
                // file's own sync_data above precedes it). Gated on config.fsync
                // so the no-fsync benchmark/offline path stays barrier-free.
                if self.config.fsync {
                    sync_dir(&self.config.dir)?;
                }
                metrics::counter_inc(metrics::SNAPSHOTS_TOTAL);
                metrics::histogram_record(
                    metrics::SNAPSHOT_DURATION_SECONDS,
                    started.elapsed().as_secs_f64(),
                );
                Ok(SnapshotFinalizeOutcome {
                    snapshot_seq: self.config.sequence,
                    body_hash: hash,
                    section_count: entries.len() as u32,
                })
            }
            Err(error) => {
                let _ = std::fs::remove_file(&tmp_path);
                Err(PersistError::Io(error))
            }
        }
    }
}

fn prepare_sections(
    sections: Vec<RawSection>,
    compression: SectionCompression,
) -> PersistResult<Vec<PreparedSection>> {
    let count = sections.len();
    let use_parallel_compression = should_prepare_compressed_sections_parallel(&sections);
    let mut prepared = match compression {
        SectionCompression::None => sections
            .into_iter()
            .map(prepare_uncompressed_section)
            .collect::<PersistResult<Vec<_>>>()?,
        SectionCompression::PerSection { level } if use_parallel_compression => sections
            .into_par_iter()
            .map(|section| prepare_compressed_section(section, level))
            .collect::<PersistResult<Vec<_>>>()?,
        SectionCompression::PerSection { level } => sections
            .into_iter()
            .map(|section| prepare_compressed_section(section, level))
            .collect::<PersistResult<Vec<_>>>()?,
    };
    assign_section_offsets(&mut prepared, count);
    Ok(prepared)
}

fn should_prepare_compressed_sections_parallel(sections: &[RawSection]) -> bool {
    if sections.len() < 2 {
        return false;
    }
    let total_bytes = sections.iter().fold(0_usize, |sum, section| {
        sum.saturating_add(section.payload.len())
    });
    total_bytes >= PARALLEL_SNAPSHOT_COMPRESSION_MIN_BYTES
}

fn prepare_uncompressed_section(section: RawSection) -> PersistResult<PreparedSection> {
    let RawSection {
        provider,
        sub,
        payload,
    } = section;
    validate_section_payload_len(payload.len())?;
    Ok(PreparedSection {
        entry: SectionEntry {
            provider,
            sub,
            payload_offset: 0,
            payload_len: payload.len() as u64,
        },
        payload,
    })
}

fn prepare_compressed_section(section: RawSection, level: i32) -> PersistResult<PreparedSection> {
    let RawSection {
        provider,
        sub,
        payload,
    } = section;
    let payload = compress_zstd(&payload, level)?;
    validate_section_payload_len(payload.len())?;
    Ok(PreparedSection {
        entry: SectionEntry {
            provider,
            sub,
            payload_offset: 0,
            payload_len: payload.len() as u64,
        },
        payload,
    })
}

fn assign_section_offsets(prepared: &mut [PreparedSection], section_count: usize) {
    let mut payload_offset =
        SNAPSHOT_FILE_HEADER_LEN as u64 + (section_count * SECTION_TABLE_ROW_LEN) as u64;
    for section in prepared {
        section.entry.payload_offset = payload_offset;
        payload_offset = payload_offset.saturating_add(section.entry.payload_len);
    }
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
        let outcome = SnapshotBuilder::new(config(dir.clone(), 1, SectionCompression::None))
            .finalize()
            .unwrap();
        assert_eq!(outcome.snapshot_seq, 1);
        let path = snapshot_path(&dir, 1);
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
        let outcome = builder.finalize().unwrap();
        assert_eq!(outcome.section_count, 1);
        let path = snapshot_path(&dir, 2);
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
        let outcome = builder.finalize().unwrap();
        assert_eq!(
            outcome.body_hash,
            SnapshotReader::open(&snapshot_path(&dir, 3))
                .unwrap()
                .header()
                .body_hash
        );
        let path = snapshot_path(&dir, 3);
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
    fn compressed_sections_parallel_gate_uses_payload_floor() {
        let single_large = [RawSection {
            provider: *b"CORE",
            sub: *b"DATA",
            payload: vec![0_u8; PARALLEL_SNAPSHOT_COMPRESSION_MIN_BYTES],
        }];
        assert!(!should_prepare_compressed_sections_parallel(&single_large));

        let below_floor = [
            RawSection {
                provider: *b"CORE",
                sub: *b"DATA",
                payload: vec![0_u8; PARALLEL_SNAPSHOT_COMPRESSION_MIN_BYTES - 1],
            },
            RawSection {
                provider: *b"CORE",
                sub: *b"IDX0",
                payload: Vec::new(),
            },
        ];
        assert!(!should_prepare_compressed_sections_parallel(&below_floor));

        let at_floor = [
            RawSection {
                provider: *b"CORE",
                sub: *b"DATA",
                payload: vec![0_u8; PARALLEL_SNAPSHOT_COMPRESSION_MIN_BYTES],
            },
            RawSection {
                provider: *b"CORE",
                sub: *b"IDX0",
                payload: Vec::new(),
            },
        ];
        assert!(should_prepare_compressed_sections_parallel(&at_floor));
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
    fn final_snapshot_path_already_exists_is_rejected_atomically() {
        let dir = temp_dir("final-exists");
        fs::write(snapshot_path(&dir, 6), b"existing").unwrap();
        let err = SnapshotBuilder::new(config(dir.clone(), 6, SectionCompression::None))
            .finalize()
            .unwrap_err();
        assert!(matches!(
            err,
            PersistError::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists
        ));
        // hard_link rejection cleans up the tmp; the pre-existing final
        // remains untouched.
        assert!(!snapshot_tmp_path(&dir, 6).exists());
        assert_eq!(fs::read(snapshot_path(&dir, 6)).unwrap(), b"existing");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn snapshot_finalize_same_sequence_is_race_safe() {
        // Two concurrent finalizes of the SAME sequence must resolve to exactly
        // one durable snapshot: one Ok, the rest a typed AlreadyExists from either
        // the tmp `create_new` or the final-path `hard_link` (the tmp path is not
        // pid/thread-unique, so both collision points are exercised). Never two
        // winners, never a panic, never a torn final file.
        use std::sync::{Arc as StdArc, Barrier};

        let dir = StdArc::new(temp_dir("same-seq-race"));
        const THREADS: usize = 6;
        let barrier = StdArc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|i| {
                let dir = StdArc::clone(&dir);
                let barrier = StdArc::clone(&barrier);
                std::thread::spawn(move || {
                    let mut builder =
                        SnapshotBuilder::new(config((*dir).clone(), 20, SectionCompression::None));
                    builder
                        .add_section(*b"CORE", *b"META", vec![i as u8; 64])
                        .unwrap();
                    barrier.wait();
                    match builder.finalize() {
                        Ok(_) => true,
                        Err(PersistError::Io(error))
                            if error.kind() == std::io::ErrorKind::AlreadyExists =>
                        {
                            false
                        }
                        Err(other) => panic!("unexpected finalize error: {other:?}"),
                    }
                })
            })
            .collect();

        let wins = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|won| *won)
            .count();
        assert_eq!(wins, 1, "exactly one same-sequence finalize may win");

        // The single durable snapshot reads back cleanly.
        let path = snapshot_path(&dir, 20);
        assert!(path.exists());
        let mut reader = SnapshotReader::open(&path).unwrap();
        reader.verify_body_hash().unwrap();
        // No tmp residue from the losing finalizes.
        assert!(!snapshot_tmp_path(&dir, 20).exists());
        let _ = fs::remove_dir_all(dir.as_path());
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

    #[test]
    fn snapshot_par_iter_roundtrip() {
        let dir = temp_dir("par-iter");
        let sections = [
            (*b"CORE", *b"META", vec![1_u8; 257 * 1024]),
            (*b"CORE", *b"NODE", vec![2_u8; 256 * 1024]),
            (*b"CORE", *b"EDGE", vec![3_u8; 256 * 1024]),
            (*b"DEMO", *b"SUBT", vec![4_u8; 256 * 1024]),
            (*b"AUX1", *b"LIST", vec![5_u8; 64 * 1024]),
        ];
        let mut builder = SnapshotBuilder::new(config(
            dir.clone(),
            9,
            SectionCompression::PerSection { level: 1 },
        ));
        for (provider, sub, payload) in &sections {
            builder
                .add_section(*provider, *sub, payload.clone())
                .unwrap();
        }
        let outcome = builder.finalize().unwrap();
        assert_eq!(outcome.section_count, sections.len() as u32);

        let mut reader = SnapshotReader::open(&snapshot_path(&dir, 9)).unwrap();
        reader.verify_body_hash().unwrap();
        assert!(reader.header().is_section_compressed());
        assert_eq!(reader.sections().len(), sections.len());
        let entries = reader.sections().to_vec();
        for ((provider, sub, expected), entry) in sections.iter().zip(&entries) {
            assert_eq!(entry.provider, *provider);
            assert_eq!(entry.sub, *sub);
            assert_eq!(
                reader.read_section(*provider, *sub).unwrap(),
                expected.as_slice()
            );
        }
        let _ = fs::remove_dir_all(dir);
    }
}
