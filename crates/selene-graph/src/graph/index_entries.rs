use std::sync::Arc;

use smallvec::SmallVec;

use selene_core::{DbString, HnswIndexConfig, IvfIndexConfig};

use crate::composite_typed_index::CompositeTypedIndex;
use crate::text_index::{TextIndex, TextIndexMemoryUsage, TextIndexStats};
use crate::typed_index::{TypedIndex, TypedIndexKind};
use crate::vector_index::{VectorIndex, VectorIndexKind, VectorIndexMemoryUsage};

/// Registered built-in property-index metadata.
#[derive(Clone, Debug)]
pub struct PropertyIndexEntry {
    /// Index data for the `(label, property)` registration.
    pub index: Arc<TypedIndex>,
    /// Optional explicit catalog name. `None` means the name is derived at render time.
    pub name: Option<DbString>,
}

impl PropertyIndexEntry {
    /// Construct an index entry from the built index and optional explicit name.
    #[must_use]
    pub fn new(index: TypedIndex, name: Option<DbString>) -> Self {
        Self {
            index: Arc::new(index),
            name,
        }
    }

    /// Return the registered index kind.
    #[must_use]
    pub fn kind(&self) -> TypedIndexKind {
        self.index.kind()
    }
}

/// Registered built-in composite-property index metadata.
#[derive(Clone, Debug)]
pub struct CompositePropertyIndexEntry {
    /// Index data for the `(label, properties...)` registration.
    pub index: Arc<CompositeTypedIndex>,
    /// Indexed properties in declaration order.
    pub declared_properties: SmallVec<[DbString; 4]>,
    /// Optional explicit catalog name. `None` means the name is derived at render time.
    pub name: Option<DbString>,
}

impl CompositePropertyIndexEntry {
    /// Construct a composite index entry.
    #[must_use]
    pub fn new(
        index: CompositeTypedIndex,
        declared_properties: SmallVec<[DbString; 4]>,
        name: Option<DbString>,
    ) -> Self {
        Self {
            index: Arc::new(index),
            declared_properties,
            name,
        }
    }

    /// Return the registered component kinds in declaration order.
    #[must_use]
    pub fn kinds(&self) -> SmallVec<[TypedIndexKind; 4]> {
        self.index.kinds().iter().copied().collect()
    }
}

/// Registered built-in vector-index metadata.
#[derive(Clone, Debug)]
pub struct VectorIndexEntry {
    /// Index data for the `(label, property)` registration.
    pub index: Arc<VectorIndex>,
    /// Optional explicit catalog name. `None` means the name is derived at render time.
    pub name: Option<DbString>,
}

impl VectorIndexEntry {
    /// Construct a vector index entry from the built index and optional name.
    #[must_use]
    pub fn new(index: VectorIndex, name: Option<DbString>) -> Self {
        Self {
            index: Arc::new(index),
            name,
        }
    }

    /// Return the registered vector index kind.
    #[must_use]
    pub fn kind(&self) -> VectorIndexKind {
        self.index.kind()
    }

    /// Return the registered vector dimensionality.
    #[must_use]
    pub fn dimension(&self) -> u32 {
        self.index.dimension()
    }

    /// Return the registered HNSW construction config, if this is an HNSW index.
    #[must_use]
    pub fn hnsw_config(&self) -> Option<HnswIndexConfig> {
        self.index.hnsw_config()
    }

    /// Return the registered IVF construction config, if this is a configured IVF index.
    #[must_use]
    pub fn ivf_config(&self) -> Option<IvfIndexConfig> {
        self.index.ivf_config()
    }

    /// Return an estimated memory usage snapshot for this vector index.
    #[must_use]
    pub fn memory_usage(&self) -> VectorIndexMemoryUsage {
        self.index.memory_usage()
    }
}

/// Registered built-in text-index metadata.
#[derive(Clone, Debug)]
pub struct TextIndexEntry {
    /// Index data for the `(label, property)` registration.
    pub index: Arc<TextIndex>,
    /// Optional explicit catalog name. `None` means the name is derived at render time.
    pub name: Option<DbString>,
}

impl TextIndexEntry {
    /// Construct a text index entry from the built index and optional name.
    #[must_use]
    pub fn new(index: TextIndex, name: Option<DbString>) -> Self {
        Self {
            index: Arc::new(index),
            name,
        }
    }

    /// Return aggregate index counters.
    #[must_use]
    pub fn stats(&self) -> TextIndexStats {
        self.index.stats()
    }

    /// Return an estimated memory usage snapshot for this text index.
    #[must_use]
    pub fn memory_usage(&self) -> TextIndexMemoryUsage {
        self.index.memory_usage()
    }
}

/// Owned row returned when iterating composite property-index registrations.
pub type CompositePropertyIndexEntryRow = (
    DbString,
    SmallVec<[DbString; 4]>,
    SmallVec<[TypedIndexKind; 4]>,
    Option<DbString>,
);

/// Owned row returned when iterating vector-index registrations.
pub type VectorIndexEntryRow = (
    DbString,
    DbString,
    VectorIndexKind,
    u32,
    Option<HnswIndexConfig>,
    Option<IvfIndexConfig>,
    Option<DbString>,
);

/// Owned row returned when iterating text-index registrations.
pub type TextIndexEntryRow = (
    DbString,
    DbString,
    TextIndexStats,
    TextIndexMemoryUsage,
    Option<DbString>,
);
