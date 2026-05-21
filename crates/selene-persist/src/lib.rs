//! Persistence formats for selene-db.
//!
//! This crate owns WAL entry headers, snapshot envelopes, section tables,
//! compression flags, and recovery streaming. It validates byte-level limits
//! before handing decoded changes or section payloads to caller-owned
//! providers, and it never owns graph state. Among selene crates it depends
//! only on `selene-core`, keeping persistence reusable below graph and runtime
//! layers. See Spec 04.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod compression;
pub mod entry_header;
pub mod error;
pub mod file_header;
mod payload;
pub mod provider;
mod reader;
pub mod recovery;
pub mod section;
pub mod snapshot_file_header;
pub mod snapshot_path;
pub mod snapshot_reader;
pub mod snapshot_writer;
mod writer;
mod writer_rotation;

pub use crate::entry_header::{
    COMPRESS_THRESHOLD, FLAG_PAYLOAD_COMPRESSED, MAX_PRINCIPAL_BYTES, MAX_WAL_ENTRY_BYTES,
    WalEntryHeader,
};
pub use crate::error::{PersistError, PersistResult};
pub use crate::file_header::{
    WAL_FILE_HEADER_LEN, WAL_MAGIC, WAL_VERSION_MAJOR, WAL_VERSION_MINOR, WalFileHeader,
};
pub use crate::provider::{ProviderRegistry, RecoveryError, RecoveryProvider, RecoveryResult};
pub use crate::reader::{WalEntry, WalEntryStream, WalEntryView, WalReader};
pub use crate::recovery::{RecoveryOutcome, recover};
pub use crate::section::{
    MAX_SECTION_COUNT, MAX_SECTION_PAYLOAD_BYTES, SECTION_TABLE_ROW_LEN, SectionEntry,
};
pub use crate::snapshot_file_header::{
    FLAG_BODY_COMPRESSED, FLAG_SECTION_COMPRESSED, SNAPSHOT_FILE_HEADER_LEN, SNAPSHOT_MAGIC,
    SNAPSHOT_VERSION_MAJOR, SNAPSHOT_VERSION_MINOR, SnapshotFileHeader,
};
pub use crate::snapshot_path::{
    SNAPSHOT_FILE_EXTENSION, SNAPSHOT_TMP_EXTENSION, find_latest_snapshot, parse_snapshot_filename,
    snapshot_path, snapshot_tmp_path,
};
pub use crate::snapshot_reader::SnapshotReader;
pub use crate::snapshot_writer::{
    SectionCompression, SnapshotBuilder, SnapshotConfig, SnapshotFinalizeOutcome,
};
pub use crate::writer::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig, WalWriter};
pub use crate::writer_rotation::WalRotationOutcome;
