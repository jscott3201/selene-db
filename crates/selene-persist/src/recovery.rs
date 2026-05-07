#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Recovery algorithm skeleton for `selene-persist`.
//!
//! D15 settles recovery as two steps. First, snapshot apply reads the latest
//! snapshot envelope and routes each `(ProviderTag, SubTag)` section to
//! `IndexProvider::read_section`. Second, WAL replay decodes entries after the
//! snapshot sequence and calls `IndexProvider::on_change` for every committed
//! change. Spec 04 owns the recovery ordering; spec 06 owns the provider trait.

use std::path::Path;

/// Outcome returned after successful recovery.
pub struct RecoveryOutcome {
    /// Snapshot sequence applied before WAL replay.
    pub applied_snapshot_seq: u64,
    /// Last WAL sequence applied.
    pub last_wal_seq: u64,
    /// Provider tags restored from the snapshot and WAL.
    pub providers_restored: Vec<ProviderTag>,
}

/// Run two-step recovery from snapshot and WAL files.
///
/// M3 will implement snapshot selection, section dispatch, WAL iteration,
/// checksum validation, and provider replay. See `_spec/04` §5 and D15.
pub fn recover(
    _snapshot_path: &Path,
    _wal_path: &Path,
    _providers: &mut [&mut dyn IndexProvider],
) -> Result<RecoveryOutcome, RecoveryError> {
    unimplemented!("M3 work - see _spec/04 §5 and D15 for the two-step algorithm")
}

/// Recovery failures.
#[derive(Debug)]
pub enum RecoveryError {
    /// Snapshot references a provider that was not registered before recovery.
    UnknownProvider {
        /// Unknown provider tag.
        tag: ProviderTag,
        /// Unknown or unroutable sub-tag under `tag`.
        sub_tag: SubTag,
    },
    /// Placeholder until M3 defines concrete failure variants.
    M3Placeholder,
}

/// Minimal provider trait placeholder for the recovery skeleton.
///
/// The canonical future trait lives in `selene-graph`; this duplicate keeps the
/// source-only skeleton independent until crate manifests exist.
pub trait IndexProvider {
    /// Return this provider's 4-byte snapshot tag.
    fn provider_tag(&self) -> ProviderTag;

    /// Apply one snapshot section to the provider's owned state.
    fn read_section(&mut self, sub_tag: SubTag, bytes: &[u8]);

    /// Observe a committed change during WAL replay.
    fn on_change(&mut self, change: &Change);
}

/// Placeholder change record; the final type lives in `selene-core`.
pub struct Change;

/// Stable 4-byte ASCII provider identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProviderTag(
    /// Provider tag bytes.
    pub [u8; 4],
);

/// Provider-owned 4-byte snapshot sub-section identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SubTag(
    /// Sub-tag bytes.
    pub [u8; 4],
);
