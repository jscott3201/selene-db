//! Persistence formats for selene-db.
//!
//! This crate owns the write-ahead log format described in
//! `_spec/04-persistence-format.md` section 3. It accepts serialized mutation
//! payloads from callers, writes append-only WAL entries, and streams entries
//! back with header filtering and lazy body decoding. It does not own graph
//! state and intentionally depends only on `selene-core` among selene crates.
//!
//! Snapshot writing and recovery orchestration are deferred to BRIEF-11 and
//! BRIEF-12.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod entry_header;
pub mod error;
pub mod file_header;
mod payload;
mod reader;
mod writer;

pub use crate::entry_header::{
    COMPRESS_THRESHOLD, FLAG_PAYLOAD_COMPRESSED, MAX_PRINCIPAL_BYTES, MAX_WAL_ENTRY_BYTES,
    WalEntryHeader,
};
pub use crate::error::{PersistError, PersistResult};
pub use crate::file_header::{
    WAL_FILE_HEADER_LEN, WAL_MAGIC, WAL_VERSION_MAJOR, WAL_VERSION_MINOR, WalFileHeader,
};
pub use crate::reader::{WalEntry, WalEntryStream, WalEntryView, WalReader};
pub use crate::writer::{DEFAULT_WAL_FILE_NAME, WalConfig, WalWriter};
