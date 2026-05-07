#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Canonical `IndexProvider` skeleton for stateful graph extensions.
//!
//! BRIEF-04a / D15 makes this module the future Rust home for the provider
//! contract described in `_spec/06-index-provider-protocol.md` §3. Recovery is
//! two-step: snapshot apply calls [`IndexProvider::read_section`] for each
//! `(ProviderTag, SubTag)` section, then WAL replay calls
//! [`IndexProvider::on_change`] for every committed [`Change`]. The persistence
//! binary section layout is defined in `_spec/04-persistence-format.md` §4.2.

/// Stateful extension hook for snapshot participation and mutation replay.
///
/// This is a skeleton only; M2/M3 will wire it to real graph and persistence
/// types. The five methods here are the complete v1.0 canonical surface from
/// spec 06 §3.
pub trait IndexProvider: Send + Sync + 'static {
    /// Return this provider's stable 4-byte ASCII provider tag.
    ///
    /// The tag forms the first half of the 8-byte snapshot composite tag. See
    /// spec 06 §3 and spec 04 §4.2.
    fn provider_tag(&self) -> ProviderTag;

    /// Decode one provider-owned snapshot section.
    ///
    /// Called during the snapshot-apply step of recovery for every section
    /// whose provider tag matches this provider. See spec 06 §3 and D15.
    fn read_section(&mut self, sub_tag: SubTag, bytes: &[u8]);

    /// Encode one provider-owned snapshot section.
    ///
    /// Called during snapshot publication for each value returned by
    /// [`Self::declared_sub_tags`]. The returned bytes are provider-private.
    /// See spec 06 §3.
    fn write_section(&self, sub_tag: SubTag) -> Vec<u8>;

    /// Observe one committed change during live commit or WAL replay.
    ///
    /// Providers match on [`Change`] and ignore unrelated variants. The
    /// `IndexExtensionEvent` variant is the provider-private WAL path. See spec
    /// 06 §3 and spec 02 §9.
    fn on_change(&mut self, change: &Change);

    /// Return the provider-owned snapshot sub-tags this provider persists.
    ///
    /// Empty means the provider participates in mutation events but has no
    /// durable snapshot state. See spec 06 §3.
    fn declared_sub_tags(&self) -> &[SubTag];
}

/// Stable 4-byte ASCII provider identifier.
///
/// Examples: `b"VECT"` for `selene-vector`, `b"META"` for core metadata.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProviderTag(
    /// Provider tag bytes.
    pub [u8; 4],
);

/// Provider-owned 4-byte snapshot sub-section identifier.
///
/// Examples: `b"BODY"` for primary content, `b"HNSW"` for vector topology.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SubTag(
    /// Sub-tag bytes.
    pub [u8; 4],
);

/// Placeholder change record; the final type lives in `selene-core`.
///
/// The real `Change` enum includes provider-private `IndexExtensionEvent`
/// records as described in spec 02 §9.
pub struct Change;
