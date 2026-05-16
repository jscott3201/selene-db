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

use crate::{HnswConfig, HnswProvider, IvfConfig, IvfProvider, VectorError, snapshot};

const DEFAULT_INDEX_NAME: &str = "default";

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

    fn default_provider(&self) -> Result<Arc<HnswProvider>, ProviderError> {
        self.get(DEFAULT_INDEX_NAME)
            .ok_or_else(|| missing_default_provider(snapshot::VECT))
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

    fn default_provider(&self) -> Result<Arc<IvfProvider>, ProviderError> {
        self.get(DEFAULT_INDEX_NAME)
            .ok_or_else(|| missing_default_provider(snapshot::IVFP))
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
}

impl IndexProvider for HnswIndexRegistry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        snapshot::VECT
    }

    fn read_section(&self, sub_tag: SubTag, bytes: &[u8]) -> Result<(), ProviderError> {
        self.default_provider()?.read_section(sub_tag, bytes)
    }

    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        self.default_provider()?.write_section(sub_tag)
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        self.default_provider()?.on_change(change)
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &snapshot::DECLARED_SUB_TAGS
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
        self.default_provider()?.read_section(sub_tag, bytes)
    }

    fn write_section(&self, sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        self.default_provider()?.write_section(sub_tag)
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        self.default_provider()?.on_change(change)
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &snapshot::DECLARED_SUB_TAGS_IVF
    }
}

fn missing_default_provider(tag: ProviderTag) -> ProviderError {
    ProviderError::Inconsistent {
        reason: format!("{tag} registry missing default vector index"),
    }
}
