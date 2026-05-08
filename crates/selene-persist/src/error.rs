//! Error types for WAL persistence.

/// Result alias for persistence operations.
pub type PersistResult<T> = Result<T, PersistError>;

/// Error type for WAL persistence operations.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum PersistError {
    /// I/O failure on the WAL file.
    #[error("wal io: {0}")]
    #[diagnostic(code(SLENE_P_001))]
    Io(#[from] std::io::Error),

    /// Postcard encode/decode failure for the entry header.
    #[error("wal header codec: {0}")]
    #[diagnostic(code(SLENE_P_002))]
    HeaderCodec(String),

    /// Postcard encode/decode failure for the payload `Vec<Change>`.
    #[error("wal payload codec: {0}")]
    #[diagnostic(code(SLENE_P_003))]
    PayloadCodec(String),

    /// Compression or decompression failure.
    #[error("wal payload compression: {0}")]
    #[diagnostic(code(SLENE_P_004))]
    Compression(String),

    /// Payload exceeds the WAL entry cap.
    #[error("wal payload too large: {len} bytes (max {max})")]
    #[diagnostic(code(SLENE_P_005))]
    PayloadTooLarge {
        /// Encoded payload length.
        len: usize,
        /// Maximum encoded payload length.
        max: usize,
    },

    /// Principal slot exceeds the D12 cap.
    #[error("wal principal too large: {len} bytes (max {max})")]
    #[diagnostic(code(SLENE_P_006))]
    PrincipalTooLarge {
        /// Principal byte length.
        len: usize,
        /// Maximum principal byte length.
        max: usize,
    },

    /// Magic bytes do not match `SLDB`.
    #[error("wal magic mismatch: expected SLDB, got {observed:?}")]
    #[diagnostic(code(SLENE_P_007))]
    MagicMismatch {
        /// Observed four-byte file magic.
        observed: [u8; 4],
    },

    /// File-header version is unsupported.
    #[error("wal version unsupported: {major}.{minor}")]
    #[diagnostic(code(SLENE_P_008))]
    UnsupportedVersion {
        /// Major version read from disk.
        major: u16,
        /// Minor version read from disk.
        minor: u16,
    },

    /// Entry checksum does not match the payload.
    #[error("wal entry checksum mismatch at sequence {sequence}")]
    #[diagnostic(code(SLENE_P_009))]
    ChecksumMismatch {
        /// WAL sequence number.
        sequence: u64,
    },

    /// Sequence numbers are not strictly monotonic.
    #[error("wal entry sequence non-monotonic: previous {previous}, current {current}")]
    #[diagnostic(code(SLENE_P_010))]
    NonMonotonicSequence {
        /// Previous valid sequence number.
        previous: u64,
        /// Current sequence number.
        current: u64,
    },

    /// File header missing or short read at open.
    #[error("wal file truncated before completing the file header")]
    #[diagnostic(code(SLENE_P_011))]
    TruncatedFileHeader,

    /// Last entry was partially written.
    #[error("wal entry truncated at offset {offset}; iterator stopped")]
    #[diagnostic(code(SLENE_P_012))]
    TruncatedEntry {
        /// File offset where the truncated entry started.
        offset: u64,
    },

    /// Another writer holds the exclusive WAL lock.
    #[error("wal writer lock is held by another process or handle")]
    #[diagnostic(code(SLENE_P_013))]
    WriterLockHeld,
}

impl PersistError {
    /// Map this error to its 5-character ISO GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> &'static str {
        match self {
            Self::PrincipalTooLarge { .. } => "22023",
            Self::PayloadTooLarge { .. } => "54000",
            Self::UnsupportedVersion { .. } => "08000",
            Self::Io(_)
            | Self::HeaderCodec(_)
            | Self::PayloadCodec(_)
            | Self::Compression(_)
            | Self::MagicMismatch { .. }
            | Self::ChecksumMismatch { .. }
            | Self::NonMonotonicSequence { .. }
            | Self::TruncatedFileHeader
            | Self::TruncatedEntry { .. }
            | Self::WriterLockHeld => "XX500",
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(PersistError::HeaderCodec("bad".to_owned()), "XX500")]
    #[case(PersistError::PayloadCodec("bad".to_owned()), "XX500")]
    #[case(PersistError::Compression("bad".to_owned()), "XX500")]
    #[case(PersistError::PayloadTooLarge { len: 1, max: 0 }, "54000")]
    #[case(PersistError::PrincipalTooLarge { len: 1, max: 0 }, "22023")]
    #[case(PersistError::MagicMismatch { observed: *b"NOPE" }, "XX500")]
    #[case(PersistError::UnsupportedVersion { major: 2, minor: 0 }, "08000")]
    #[case(PersistError::ChecksumMismatch { sequence: 7 }, "XX500")]
    #[case(PersistError::NonMonotonicSequence { previous: 7, current: 7 }, "XX500")]
    #[case(PersistError::TruncatedFileHeader, "XX500")]
    #[case(PersistError::TruncatedEntry { offset: 16 }, "XX500")]
    #[case(PersistError::WriterLockHeld, "XX500")]
    fn gqlstatus_for_each_variant(#[case] error: PersistError, #[case] status: &str) {
        assert_eq!(error.gqlstatus(), status);
    }
}
