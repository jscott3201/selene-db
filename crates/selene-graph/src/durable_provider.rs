//! Commit-critical durable provider protocol.

use std::sync::Arc;

use selene_core::{Change, HlcTimestamp};

use crate::index_provider::{ProviderError, ProviderTag};

/// Durable sink invoked before a graph snapshot is published.
///
/// Unlike [`crate::IndexProvider`], failures from this trait abort the commit.
/// Implementors must not call back into the same graph while handling a commit;
/// the writer still owns the graph write lock.
pub trait DurableProvider: Send + Sync + 'static {
    /// Stable 4-byte ASCII tag identifying this durable sink.
    fn provider_tag(&self) -> ProviderTag;

    /// Allocate the timestamp shared by every durable sink for one commit.
    ///
    /// Implementors without their own monotonic clock may keep the default zero
    /// timestamp. The built-in CORE WAL sink overrides this to seed from the WAL
    /// sequence watermark and advance once per commit.
    fn next_timestamp(&self) -> HlcTimestamp {
        HlcTimestamp::zero()
    }

    /// Persist one committed change set before the graph snapshot is published.
    ///
    /// Returns the durable sequence number assigned by this provider.
    ///
    /// `principal` is borrowed as `Option<&Arc<[u8]>>` so an implementor that
    /// retains the bytes (e.g. the CORE WAL header) clones the existing `Arc`
    /// (a refcount bump) rather than re-allocating and copying the slice.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the durable write fails. The caller aborts
    /// the commit and rolls the graph state back.
    fn write_commit(
        &self,
        principal: Option<&Arc<[u8]>>,
        changes: &[Change],
        timestamp: HlcTimestamp,
    ) -> Result<u64, ProviderError>;

    /// Force provider-owned durable state to stable storage.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the flush fails.
    fn flush(&self) -> Result<Option<u64>, ProviderError> {
        Ok(None)
    }
}
