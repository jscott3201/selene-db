//! In-memory activation occupancy registry.

use std::collections::BTreeMap;

use jiff::Timestamp;

use super::{Active, ContentHash, Principal};
use crate::ProcedurePackManifest;

/// Activation occupancy status for a pack name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ActivationStatus {
    /// Pack is active.
    Active,
    /// Pack is deprecated but still occupies its name.
    Deprecated,
}

/// Read-only snapshot of an activation registry entry.
#[derive(Clone, Debug)]
pub struct ActivationEntry {
    pack_name: String,
    manifest: ProcedurePackManifest,
    content_hash: ContentHash,
    principal: Principal,
    status: ActivationStatus,
    activated_at: Timestamp,
    deprecated_at: Option<Timestamp>,
}

impl ActivationEntry {
    pub(crate) fn active(
        manifest: ProcedurePackManifest,
        content_hash: ContentHash,
        principal: Principal,
        activated_at: Timestamp,
    ) -> Self {
        let pack_name = manifest.pack_name.clone();
        Self {
            pack_name,
            manifest,
            content_hash,
            principal,
            status: ActivationStatus::Active,
            activated_at,
            deprecated_at: None,
        }
    }

    pub(crate) fn deprecated_from_active(active: &Active, deprecated_at: Timestamp) -> Self {
        Self {
            pack_name: active.pack_name().to_owned(),
            manifest: active.manifest().clone(),
            content_hash: active.content_hash(),
            principal: active.principal(),
            status: ActivationStatus::Deprecated,
            activated_at: active.activated_at(),
            deprecated_at: Some(deprecated_at),
        }
    }

    /// Return the pack name.
    #[must_use]
    pub fn pack_name(&self) -> &str {
        &self.pack_name
    }

    /// Return the validated manifest snapshot.
    #[must_use]
    pub fn manifest(&self) -> &ProcedurePackManifest {
        &self.manifest
    }

    /// Return the pack content hash.
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    /// Return the principal that activated the pack.
    #[must_use]
    pub const fn principal(&self) -> Principal {
        self.principal
    }

    /// Return the current occupancy status.
    #[must_use]
    pub const fn status(&self) -> ActivationStatus {
        self.status
    }

    /// Return the activation timestamp.
    #[must_use]
    pub const fn activated_at(&self) -> Timestamp {
        self.activated_at
    }

    /// Return the deprecation timestamp, if this pack has been deprecated.
    #[must_use]
    pub const fn deprecated_at(&self) -> Option<Timestamp> {
        self.deprecated_at
    }
}

/// In-memory activation occupancy registry keyed by pack name.
#[derive(Clone, Debug, Default)]
pub struct ActivationRegistry {
    entries: BTreeMap<String, ActivationEntry>,
}

impl ActivationRegistry {
    /// Construct an empty activation registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Return true when no pack names are occupied.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return the number of occupied pack names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return true when the pack name is active or deprecated.
    #[must_use]
    pub fn is_occupied(&self, pack_name: &str) -> bool {
        self.entries.contains_key(pack_name)
    }

    /// Return true when the pack name is active.
    #[must_use]
    pub fn is_active(&self, pack_name: &str) -> bool {
        self.get(pack_name)
            .is_some_and(|entry| entry.status() == ActivationStatus::Active)
    }

    /// Return true when the pack name is deprecated.
    #[must_use]
    pub fn is_deprecated(&self, pack_name: &str) -> bool {
        self.get(pack_name)
            .is_some_and(|entry| entry.status() == ActivationStatus::Deprecated)
    }

    /// Return the activation entry for a pack name.
    #[must_use]
    pub fn get(&self, pack_name: &str) -> Option<&ActivationEntry> {
        self.entries.get(pack_name)
    }

    /// Iterate activation entries in lexicographic pack-name order.
    pub fn iter(&self) -> impl Iterator<Item = &ActivationEntry> {
        self.entries.values()
    }

    pub(crate) fn insert_active(&mut self, entry: ActivationEntry) {
        self.entries.insert(entry.pack_name.clone(), entry);
    }

    pub(crate) fn mark_deprecated(&mut self, entry: ActivationEntry) {
        self.entries.insert(entry.pack_name.clone(), entry);
    }

    pub(crate) fn remove(&mut self, pack_name: &str) {
        self.entries.remove(pack_name);
    }
}
