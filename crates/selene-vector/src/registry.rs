//! Multi-index vector registry layer (BRIEF-109).
//!
//! PR1 establishes the registry types and re-routes the existing single-default
//! path through them, preserving v1.0 wire format and behavior byte-for-byte.
//! PR2 adds the lifecycle procedures (`vector.create_index` / `drop_index` /
//! `list_indexes`) and the named-index wire-format extension. Existing
//! embedders may keep registering bare [`HnswProvider`] and [`IvfProvider`]
//! values; new embedders can register these registries as [`IndexProvider`]s.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use selene_core::Change;
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SubTag};

use crate::config::DistanceMetric;
use crate::payload::{LifecycleEventKind, decode_lifecycle_event, split_named_payload};
use crate::snapshot::wrapper::{
    HnswSnapshotEntry, IvfSnapshotEntry, decode_hnsw_entries, decode_ivf_entries,
    encode_hnsw_entries, encode_ivf_entries,
};
use crate::{HnswConfig, HnswProvider, IvfConfig, IvfProvider, VectorError, snapshot};

mod coverage;
use coverage::validate_snapshot_wrapper_coverage;

const DEFAULT_INDEX_NAME: &str = "default";
const LIFECYCLE_PROVIDER_NAME: &str = "selene-vector-lifecycle";
const MAX_INDEX_NAME_BYTES: usize = u16::MAX as usize;

/// Kind discriminator for named vector indexes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorIndexKind {
    /// HNSW index under the `VECT` provider tag.
    Hnsw,
    /// IVF-PQ index under the `IVFP` provider tag.
    Ivf,
}

/// Result of a lifecycle create operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateIndexOutcome {
    /// A new provider was inserted into the target registry.
    Created,
    /// The same normalized config already existed for this name and kind.
    AlreadyExists,
}

/// Result of a lifecycle drop operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DropIndexOutcome {
    /// An HNSW provider was removed.
    Hnsw,
    /// An IVF provider was removed.
    Ivf,
    /// No provider existed for that name.
    Missing,
}

/// One row returned by registry listing.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexListEntry {
    /// Index name.
    pub name: Arc<str>,
    /// Index kind.
    pub kind: VectorIndexKind,
    /// Configured vector dimensionality.
    pub dim: usize,
    /// Configured distance metric.
    pub metric: DistanceMetric,
    /// Current number of vectors in the provider.
    pub vector_count: usize,
}

/// Registry for HNSW vector indexes under the `VECT` provider tag.
pub struct HnswIndexRegistry {
    entries: RwLock<HashMap<Arc<str>, Arc<HnswProvider>>>,
}

/// Registry for IVF-PQ vector indexes under the `IVFP` provider tag.
pub struct IvfIndexRegistry {
    entries: RwLock<HashMap<Arc<str>, Arc<IvfProvider>>>,
}

/// Shared catalog handle reserved for BRIEF-109 lifecycle coordination.
#[allow(dead_code)]
pub struct Catalog {
    pub(crate) hnsw: Arc<HnswIndexRegistry>,
    pub(crate) ivf: Arc<IvfIndexRegistry>,
}

impl HnswIndexRegistry {
    /// Construct a registry seeded with the compatibility `"default"` index.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when the default HNSW provider cannot be
    /// constructed from `default_config`.
    pub fn new(default_config: HnswConfig) -> Result<Self, VectorError> {
        let default_provider = Arc::new(HnswProvider::new(default_config)?);
        Ok(Self::from_default_provider(default_provider))
    }

    /// Construct a registry from an already-created default provider.
    #[must_use]
    pub fn from_default_provider(default_provider: Arc<HnswProvider>) -> Self {
        let mut entries = HashMap::with_capacity(1);
        entries.insert(Arc::<str>::from(DEFAULT_INDEX_NAME), default_provider);
        Self {
            entries: RwLock::new(entries),
        }
    }

    /// Return the provider registered for `name`, if present.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<HnswProvider>> {
        let entries = self.entries.read();
        entries.get(name).cloned()
    }

    /// Return the compatibility default provider config.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when the registry no longer contains the
    /// default provider.
    pub fn default_config(&self) -> Result<HnswConfig, VectorError> {
        self.get(DEFAULT_INDEX_NAME)
            .map(|provider| provider.config().clone())
            .ok_or_else(|| registry_invalid("HNSW registry missing default vector index"))
    }

    /// Return sorted provider entries for snapshot emission.
    #[must_use]
    pub fn entries(&self) -> Vec<(Arc<str>, Arc<HnswProvider>)> {
        let entries = self.entries.read();
        let mut out = entries
            .iter()
            .map(|(name, provider)| (Arc::clone(name), Arc::clone(provider)))
            .collect::<Vec<_>>();
        out.sort_by(|(left, _), (right, _)| left.cmp(right));
        out
    }

    fn create_local(
        &self,
        name: &str,
        config: HnswConfig,
    ) -> Result<CreateIndexOutcome, VectorError> {
        validate_index_name(name)?;
        let mut entries = self.entries.write();
        if let Some(existing) = entries.get(name) {
            if existing.config() == &config {
                return Ok(CreateIndexOutcome::AlreadyExists);
            }
            return Err(registry_invalid(format!(
                "vector index '{name}' already exists with a different HNSW config"
            )));
        }
        entries.insert(Arc::<str>::from(name), Arc::new(HnswProvider::new(config)?));
        Ok(CreateIndexOutcome::Created)
    }

    fn drop_local(&self, name: &str) -> Result<DropIndexOutcome, VectorError> {
        validate_index_name(name)?;
        if name == DEFAULT_INDEX_NAME {
            return Err(registry_invalid("cannot drop the default vector index"));
        }
        let mut entries = self.entries.write();
        Ok(if entries.remove(name).is_some() {
            DropIndexOutcome::Hnsw
        } else {
            DropIndexOutcome::Missing
        })
    }
}

impl IvfIndexRegistry {
    /// Construct a registry seeded with the compatibility `"default"` index.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when the default IVF provider cannot be
    /// constructed from `default_config`.
    pub fn new(default_config: IvfConfig) -> Result<Self, VectorError> {
        let default_provider = Arc::new(IvfProvider::new(default_config)?);
        Ok(Self::from_default_provider(default_provider))
    }

    /// Construct a registry from an already-created default provider.
    #[must_use]
    pub fn from_default_provider(default_provider: Arc<IvfProvider>) -> Self {
        let mut entries = HashMap::with_capacity(1);
        entries.insert(Arc::<str>::from(DEFAULT_INDEX_NAME), default_provider);
        Self {
            entries: RwLock::new(entries),
        }
    }

    /// Return the provider registered for `name`, if present.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<IvfProvider>> {
        let entries = self.entries.read();
        entries.get(name).cloned()
    }

    /// Return the compatibility default provider config.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when the registry no longer contains the
    /// default provider.
    pub fn default_config(&self) -> Result<IvfConfig, VectorError> {
        self.get(DEFAULT_INDEX_NAME)
            .map(|provider| *provider.config())
            .ok_or_else(|| registry_invalid("IVF registry missing default vector index"))
    }

    /// Return sorted provider entries for snapshot emission.
    #[must_use]
    pub fn entries(&self) -> Vec<(Arc<str>, Arc<IvfProvider>)> {
        let entries = self.entries.read();
        let mut out = entries
            .iter()
            .map(|(name, provider)| (Arc::clone(name), Arc::clone(provider)))
            .collect::<Vec<_>>();
        out.sort_by(|(left, _), (right, _)| left.cmp(right));
        out
    }

    fn create_local(
        &self,
        name: &str,
        config: IvfConfig,
    ) -> Result<CreateIndexOutcome, VectorError> {
        validate_index_name(name)?;
        let mut entries = self.entries.write();
        if let Some(existing) = entries.get(name) {
            if existing.config() == &config {
                return Ok(CreateIndexOutcome::AlreadyExists);
            }
            return Err(registry_invalid(format!(
                "vector index '{name}' already exists with a different IVF config"
            )));
        }
        entries.insert(Arc::<str>::from(name), Arc::new(IvfProvider::new(config)?));
        Ok(CreateIndexOutcome::Created)
    }

    fn drop_local(&self, name: &str) -> Result<DropIndexOutcome, VectorError> {
        validate_index_name(name)?;
        if name == DEFAULT_INDEX_NAME {
            return Err(registry_invalid("cannot drop the default vector index"));
        }
        let mut entries = self.entries.write();
        Ok(if entries.remove(name).is_some() {
            DropIndexOutcome::Ivf
        } else {
            DropIndexOutcome::Missing
        })
    }
}

impl Catalog {
    /// Construct a catalog with default HNSW and IVF registries.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when either default provider cannot be
    /// constructed from its config.
    pub fn new(
        hnsw_default_config: HnswConfig,
        ivf_default_config: IvfConfig,
    ) -> Result<Self, VectorError> {
        Ok(Self {
            hnsw: Arc::new(HnswIndexRegistry::new(hnsw_default_config)?),
            ivf: Arc::new(IvfIndexRegistry::new(ivf_default_config)?),
        })
    }

    /// Construct a catalog from already-created registries.
    #[must_use]
    pub fn from_registries(hnsw: Arc<HnswIndexRegistry>, ivf: Arc<IvfIndexRegistry>) -> Self {
        Self { hnsw, ivf }
    }

    /// Borrow the HNSW registry.
    #[must_use]
    pub fn hnsw(&self) -> &Arc<HnswIndexRegistry> {
        &self.hnsw
    }

    /// Borrow the IVF registry.
    #[must_use]
    pub fn ivf(&self) -> &Arc<IvfIndexRegistry> {
        &self.ivf
    }

    /// Create or idempotently confirm an HNSW index.
    ///
    /// Non-default names are globally unique across the HNSW and IVF
    /// registries. `"default"` is the compatibility exception.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when the name conflicts with another kind, the
    /// existing config differs, or the provider cannot be built.
    pub fn create_hnsw_index(
        &self,
        name: &str,
        config: HnswConfig,
    ) -> Result<CreateIndexOutcome, VectorError> {
        Self::create_hnsw_index_in(&self.hnsw, &self.ivf, name, config)
    }

    /// Create or idempotently confirm an HNSW index across borrowed
    /// registries.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when the name conflicts with another kind, the
    /// existing config differs, or the provider cannot be built.
    pub fn create_hnsw_index_in(
        hnsw_registry: &HnswIndexRegistry,
        ivf_registry: &IvfIndexRegistry,
        name: &str,
        config: HnswConfig,
    ) -> Result<CreateIndexOutcome, VectorError> {
        validate_index_name(name)?;
        // Lock order: hnsw before ivf.
        let mut hnsw = hnsw_registry.entries.write();
        let ivf = ivf_registry.entries.read();
        if name != DEFAULT_INDEX_NAME && ivf.contains_key(name) {
            return Err(registry_invalid(format!(
                "vector index '{name}' already exists with kind ivf"
            )));
        }
        if let Some(existing) = hnsw.get(name) {
            if existing.config() == &config {
                return Ok(CreateIndexOutcome::AlreadyExists);
            }
            return Err(registry_invalid(format!(
                "vector index '{name}' already exists with a different HNSW config"
            )));
        }
        let provider = Arc::new(HnswProvider::new(config)?);
        hnsw.insert(Arc::<str>::from(name), provider);
        Ok(CreateIndexOutcome::Created)
    }

    /// Create or idempotently confirm an IVF index.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when the name conflicts with another kind, the
    /// existing config differs, or the provider cannot be built.
    pub fn create_ivf_index(
        &self,
        name: &str,
        config: IvfConfig,
    ) -> Result<CreateIndexOutcome, VectorError> {
        Self::create_ivf_index_in(&self.hnsw, &self.ivf, name, config)
    }

    /// Create or idempotently confirm an IVF index across borrowed registries.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when the name conflicts with another kind, the
    /// existing config differs, or the provider cannot be built.
    pub fn create_ivf_index_in(
        hnsw_registry: &HnswIndexRegistry,
        ivf_registry: &IvfIndexRegistry,
        name: &str,
        config: IvfConfig,
    ) -> Result<CreateIndexOutcome, VectorError> {
        validate_index_name(name)?;
        // Lock order: hnsw before ivf.
        let hnsw = hnsw_registry.entries.read();
        let mut ivf = ivf_registry.entries.write();
        if name != DEFAULT_INDEX_NAME && hnsw.contains_key(name) {
            return Err(registry_invalid(format!(
                "vector index '{name}' already exists with kind hnsw"
            )));
        }
        if let Some(existing) = ivf.get(name) {
            if existing.config() == &config {
                return Ok(CreateIndexOutcome::AlreadyExists);
            }
            return Err(registry_invalid(format!(
                "vector index '{name}' already exists with a different IVF config"
            )));
        }
        let provider = Arc::new(IvfProvider::new(config)?);
        ivf.insert(Arc::<str>::from(name), provider);
        Ok(CreateIndexOutcome::Created)
    }

    /// Drop a non-default index from whichever registry owns its globally
    /// unique name.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when `name` is `"default"`.
    pub fn drop_index(&self, name: &str) -> Result<DropIndexOutcome, VectorError> {
        Self::drop_index_in(&self.hnsw, &self.ivf, name)
    }

    /// Drop a non-default index from borrowed registries.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError`] when `name` is `"default"`.
    pub fn drop_index_in(
        hnsw_registry: &HnswIndexRegistry,
        ivf_registry: &IvfIndexRegistry,
        name: &str,
    ) -> Result<DropIndexOutcome, VectorError> {
        validate_index_name(name)?;
        if name == DEFAULT_INDEX_NAME {
            return Err(registry_invalid("cannot drop the default vector index"));
        }
        // Lock order: hnsw before ivf.
        let mut hnsw = hnsw_registry.entries.write();
        let mut ivf = ivf_registry.entries.write();
        if hnsw.remove(name).is_some() {
            return Ok(DropIndexOutcome::Hnsw);
        }
        if ivf.remove(name).is_some() {
            return Ok(DropIndexOutcome::Ivf);
        }
        Ok(DropIndexOutcome::Missing)
    }

    /// Return the kind registered for `name`, if present.
    #[must_use]
    pub fn lookup_kind(&self, name: &str) -> Option<VectorIndexKind> {
        Self::lookup_kind_in(&self.hnsw, &self.ivf, name)
    }

    /// Return the kind registered for `name` in borrowed registries.
    #[must_use]
    pub fn lookup_kind_in(
        hnsw_registry: &HnswIndexRegistry,
        ivf_registry: &IvfIndexRegistry,
        name: &str,
    ) -> Option<VectorIndexKind> {
        // Lock order: hnsw before ivf.
        let hnsw = hnsw_registry.entries.read();
        let ivf = ivf_registry.entries.read();
        if hnsw.contains_key(name) {
            return Some(VectorIndexKind::Hnsw);
        }
        if ivf.contains_key(name) {
            return Some(VectorIndexKind::Ivf);
        }
        None
    }

    /// Return a sorted list of both registry contents.
    #[must_use]
    pub fn list_indexes(&self) -> Vec<IndexListEntry> {
        Self::list_indexes_in(&self.hnsw, &self.ivf)
    }

    /// Return a sorted list of borrowed registry contents.
    #[must_use]
    pub fn list_indexes_in(
        hnsw_registry: &HnswIndexRegistry,
        ivf_registry: &IvfIndexRegistry,
    ) -> Vec<IndexListEntry> {
        // Lock order: hnsw before ivf.
        let hnsw = hnsw_registry.entries.read();
        let ivf = ivf_registry.entries.read();
        let mut entries = Vec::with_capacity(hnsw.len() + ivf.len());
        entries.extend(hnsw.iter().map(|(name, provider)| IndexListEntry {
            name: Arc::clone(name),
            kind: VectorIndexKind::Hnsw,
            dim: provider.config().dim,
            metric: provider.config().metric,
            vector_count: provider.snapshot().len(),
        }));
        entries.extend(ivf.iter().map(|(name, provider)| IndexListEntry {
            name: Arc::clone(name),
            kind: VectorIndexKind::Ivf,
            dim: provider.config().dim,
            metric: provider.config().metric,
            vector_count: provider.snapshot().len(),
        }));
        entries.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| kind_order(left.kind).cmp(&kind_order(right.kind)))
        });
        entries
    }
}

impl IndexProvider for HnswIndexRegistry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        snapshot::VECT
    }

    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        self.read_wrapped_section(sub_tag, bytes)
    }

    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        self.write_wrapped_section(sub_tag)
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        let Change::IndexExtensionEvent { provider, payload } = change else {
            return Ok(());
        };
        match provider.as_str() {
            crate::provider::PROVIDER_NAME => {
                let named = split_named_payload(payload.as_ref()).map_err(payload_decode_err)?;
                let hnsw =
                    self.get(&named.index_name)
                        .ok_or_else(|| ProviderError::Inconsistent {
                            reason: format!(
                                "VECT registry has no vector index '{}'",
                                named.index_name
                            ),
                        })?;
                let stripped = Change::IndexExtensionEvent {
                    provider: *provider,
                    payload: Arc::from(named.body.into_boxed_slice()),
                };
                hnsw.on_change(&stripped)
            }
            LIFECYCLE_PROVIDER_NAME => self.on_lifecycle(payload.as_ref()),
            _ => Ok(()),
        }
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &snapshot::DECLARED_SUB_TAGS
    }
}

impl HnswIndexRegistry {
    fn read_wrapped_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        if !snapshot::is_declared(sub_tag) {
            return Err(unknown_sub_tag(snapshot::VECT, sub_tag));
        }
        let default_config = self.default_config().map_err(registry_provider_err)?;
        let entries =
            decode_hnsw_entries(sub_tag, bytes, default_config).map_err(section_decode_err)?;
        if sub_tag == snapshot::GRPH {
            let mut next = HashMap::with_capacity(entries.len());
            for entry in entries {
                let provider = Arc::new(
                    HnswProvider::new(entry.config.clone()).map_err(registry_provider_err)?,
                );
                provider.read_section(sub_tag, &entry.section)?;
                next.insert(entry.name, provider);
            }
            if !next.contains_key(DEFAULT_INDEX_NAME) {
                return Err(ProviderError::InvalidPayload {
                    reason: "VECT GRPH wrapper missing 'default' entry".to_owned(),
                });
            }
            *self.entries.write() = next;
            return Ok(());
        }
        let staged_names = self.entries.read().keys().cloned().collect::<Vec<_>>();
        validate_snapshot_wrapper_coverage(
            sub_tag,
            staged_names,
            entries.iter().map(|entry| entry.name.as_ref()),
        )?;
        for entry in entries {
            let provider = self
                .get(&entry.name)
                .ok_or_else(|| ProviderError::InvalidPayload {
                    reason: format!(
                        "{sub_tag} snapshot entry '{}' has no staged GRPH provider",
                        entry.name
                    ),
                })?;
            if provider.config() != &entry.config {
                return Err(ProviderError::InvalidPayload {
                    reason: format!(
                        "{sub_tag} snapshot entry '{}' config disagrees with GRPH",
                        entry.name
                    ),
                });
            }
            provider.read_section(sub_tag, &entry.section)?;
        }
        Ok(())
    }

    fn write_wrapped_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        if !snapshot::is_declared(sub_tag) {
            return Err(unknown_sub_tag(snapshot::VECT, sub_tag));
        }
        let entries = self
            .entries()
            .into_iter()
            .map(|(name, provider)| {
                provider
                    .write_section(sub_tag)
                    .map(|section| HnswSnapshotEntry {
                        name,
                        config: provider.config().clone(),
                        section,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        encode_hnsw_entries(sub_tag, entries).map_err(section_encode_err)
    }

    fn on_lifecycle(&self, payload: &[u8]) -> Result<(), ProviderError> {
        let named = split_named_payload(payload).map_err(payload_decode_err)?;
        match decode_lifecycle_event(&named.body).map_err(payload_decode_err)? {
            LifecycleEventKind::Create(event) if event.kind == "hnsw" => {
                let config = decode_hnsw_config(&event.config).map_err(payload_decode_err)?;
                self.create_local(&named.index_name, config)
                    .map_err(registry_provider_err)?;
                Ok(())
            }
            LifecycleEventKind::Create(_) => Ok(()),
            LifecycleEventKind::Drop(_) => {
                self.drop_local(&named.index_name)
                    .map_err(registry_provider_err)?;
                Ok(())
            }
        }
    }
}

impl IndexProvider for IvfIndexRegistry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        snapshot::IVFP
    }

    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        self.read_wrapped_section(sub_tag, bytes)
    }

    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        self.write_wrapped_section(sub_tag)
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        let Change::IndexExtensionEvent { provider, payload } = change else {
            return Ok(());
        };
        match provider.as_str() {
            crate::ivf::IVF_PROVIDER_NAME => {
                let named = split_named_payload(payload.as_ref()).map_err(payload_decode_err)?;
                let ivf =
                    self.get(&named.index_name)
                        .ok_or_else(|| ProviderError::Inconsistent {
                            reason: format!(
                                "IVFP registry has no vector index '{}'",
                                named.index_name
                            ),
                        })?;
                let stripped = Change::IndexExtensionEvent {
                    provider: *provider,
                    payload: Arc::from(named.body.into_boxed_slice()),
                };
                ivf.on_change(&stripped)
            }
            LIFECYCLE_PROVIDER_NAME => self.on_lifecycle(payload.as_ref()),
            _ => Ok(()),
        }
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &snapshot::DECLARED_SUB_TAGS_IVF
    }
}

impl IvfIndexRegistry {
    fn read_wrapped_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        if !snapshot::is_declared_ivf(sub_tag) {
            return Err(unknown_sub_tag(snapshot::IVFP, sub_tag));
        }
        let default_config = self.default_config().map_err(registry_provider_err)?;
        let entries =
            decode_ivf_entries(sub_tag, bytes, default_config).map_err(section_decode_err)?;
        if sub_tag == snapshot::CQNT {
            let mut next = HashMap::with_capacity(entries.len());
            for entry in entries {
                let provider =
                    Arc::new(IvfProvider::new(entry.config).map_err(registry_provider_err)?);
                provider.read_section(sub_tag, &entry.section)?;
                next.insert(entry.name, provider);
            }
            if !next.contains_key(DEFAULT_INDEX_NAME) {
                return Err(ProviderError::InvalidPayload {
                    reason: "IVFP CQNT wrapper missing 'default' entry".to_owned(),
                });
            }
            *self.entries.write() = next;
            return Ok(());
        }
        let staged_names = self.entries.read().keys().cloned().collect::<Vec<_>>();
        validate_snapshot_wrapper_coverage(
            sub_tag,
            staged_names,
            entries.iter().map(|entry| entry.name.as_ref()),
        )?;
        for entry in entries {
            let provider = self
                .get(&entry.name)
                .ok_or_else(|| ProviderError::InvalidPayload {
                    reason: format!(
                        "{sub_tag} snapshot entry '{}' has no staged CQNT provider",
                        entry.name
                    ),
                })?;
            if provider.config() != &entry.config {
                return Err(ProviderError::InvalidPayload {
                    reason: format!(
                        "{sub_tag} snapshot entry '{}' config disagrees with CQNT",
                        entry.name
                    ),
                });
            }
            provider.read_section(sub_tag, &entry.section)?;
        }
        Ok(())
    }

    fn write_wrapped_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        if !snapshot::is_declared_ivf(sub_tag) {
            return Err(unknown_sub_tag(snapshot::IVFP, sub_tag));
        }
        let entries = self
            .entries()
            .into_iter()
            .map(|(name, provider)| {
                provider
                    .write_section(sub_tag)
                    .map(|section| IvfSnapshotEntry {
                        name,
                        config: *provider.config(),
                        section,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        encode_ivf_entries(sub_tag, entries).map_err(section_encode_err)
    }

    fn on_lifecycle(&self, payload: &[u8]) -> Result<(), ProviderError> {
        let named = split_named_payload(payload).map_err(payload_decode_err)?;
        match decode_lifecycle_event(&named.body).map_err(payload_decode_err)? {
            LifecycleEventKind::Create(event) if event.kind == "ivf" => {
                let config = decode_ivf_config(&event.config).map_err(payload_decode_err)?;
                self.create_local(&named.index_name, config)
                    .map_err(registry_provider_err)?;
                Ok(())
            }
            LifecycleEventKind::Create(_) => Ok(()),
            LifecycleEventKind::Drop(_) => {
                self.drop_local(&named.index_name)
                    .map_err(registry_provider_err)?;
                Ok(())
            }
        }
    }
}

fn unknown_sub_tag(tag: ProviderTag, sub_tag: SubTag) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: format!("unknown {tag} registry sub_tag {sub_tag}"),
    }
}

fn section_decode_err(err: VectorError) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: format!("vector registry section decode: {err:?}: {err}"),
    }
}

fn section_encode_err(err: VectorError) -> ProviderError {
    ProviderError::SerializationFailed {
        reason: format!("vector registry section encode: {err:?}: {err}"),
    }
}

/// Encode a normalized HNSW config for lifecycle WAL and snapshot metadata.
///
/// # Errors
///
/// Returns [`VectorError::EncodeFailed`] if serialization fails.
pub fn encode_hnsw_config(config: &HnswConfig) -> Result<Vec<u8>, VectorError> {
    postcard::to_allocvec(config).map_err(|error| VectorError::EncodeFailed {
        reason: format!("HNSW config encode failed: {error}"),
    })
}

/// Decode a normalized HNSW config from lifecycle WAL or snapshot metadata.
///
/// # Errors
///
/// Returns [`VectorError::InvalidPayload`] when the bytes do not decode to a
/// valid config.
pub fn decode_hnsw_config(bytes: &[u8]) -> Result<HnswConfig, VectorError> {
    let config =
        postcard::from_bytes::<HnswConfig>(bytes).map_err(|error| VectorError::InvalidPayload {
            reason: format!("HNSW config decode failed: {error}"),
        })?;
    config.validate()?;
    Ok(config)
}

/// Encode a normalized IVF config for lifecycle WAL and snapshot metadata.
///
/// # Errors
///
/// Returns [`VectorError::EncodeFailed`] if serialization fails.
pub fn encode_ivf_config(config: &IvfConfig) -> Result<Vec<u8>, VectorError> {
    postcard::to_allocvec(config).map_err(|error| VectorError::EncodeFailed {
        reason: format!("IVF config encode failed: {error}"),
    })
}

/// Decode a normalized IVF config from lifecycle WAL or snapshot metadata.
///
/// # Errors
///
/// Returns [`VectorError::InvalidPayload`] when the bytes do not decode to a
/// valid config.
pub fn decode_ivf_config(bytes: &[u8]) -> Result<IvfConfig, VectorError> {
    let config =
        postcard::from_bytes::<IvfConfig>(bytes).map_err(|error| VectorError::InvalidPayload {
            reason: format!("IVF config decode failed: {error}"),
        })?;
    config.validate()?;
    Ok(config)
}

fn validate_index_name(name: &str) -> Result<(), VectorError> {
    if name.is_empty() {
        return Err(registry_invalid("vector index name must be non-empty"));
    }
    if name.len() > MAX_INDEX_NAME_BYTES {
        return Err(registry_invalid(format!(
            "vector index name exceeds {MAX_INDEX_NAME_BYTES}-byte wire limit"
        )));
    }
    Ok(())
}

fn registry_invalid(reason: impl Into<String>) -> VectorError {
    VectorError::InvalidConfig {
        reason: reason.into(),
    }
}

fn kind_order(kind: VectorIndexKind) -> u8 {
    match kind {
        VectorIndexKind::Hnsw => 0,
        VectorIndexKind::Ivf => 1,
    }
}

fn payload_decode_err(err: VectorError) -> ProviderError {
    ProviderError::InvalidPayload {
        reason: format!("vector registry payload decode: {err:?}: {err}"),
    }
}

fn registry_provider_err(err: VectorError) -> ProviderError {
    ProviderError::Inconsistent {
        reason: format!("vector registry lifecycle failed: {err:?}: {err}"),
    }
}
