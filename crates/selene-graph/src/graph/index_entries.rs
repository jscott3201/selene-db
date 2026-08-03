use std::borrow::Cow;
use std::ops::RangeBounds;
use std::sync::Arc;

use roaring::RoaringBitmap;
use smallvec::SmallVec;

use selene_core::{DbString, HnswIndexConfig, IvfIndexConfig, Value};

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
    /// Live rows omitted from `index` because the stored value's variant does
    /// not match the registered index kind.
    ///
    /// Open graphs accept a value of any variant, so a column carrying an
    /// index can hold rows the index cannot key. While this is non-zero the
    /// index is an incomplete view of the column and every probe declines, so
    /// callers fall back to a scan and see the same rows either way.
    ///
    /// NaN skips are excluded. NaN satisfies no equality or range predicate, so
    /// a scan omits a NaN row too and the index remains complete without it.
    pub drifted_rows: u64,
}

impl PropertyIndexEntry {
    /// Construct an index entry from the built index and optional explicit name.
    ///
    /// The entry is treated as a complete view of its column. Use
    /// [`PropertyIndexEntry::new_with_drift`] for a lenient rebuild, whose
    /// tally must survive into the entry or a recovered graph would answer
    /// queries differently from the live one it replaced.
    #[must_use]
    pub fn new(index: TypedIndex, name: Option<DbString>) -> Self {
        Self::new_with_drift(index, name, 0)
    }

    /// Construct an index entry carrying a lenient build's drift tally.
    #[must_use]
    pub fn new_with_drift(index: TypedIndex, name: Option<DbString>, drifted_rows: u64) -> Self {
        Self {
            index: Arc::new(index),
            name,
            drifted_rows,
        }
    }

    /// Return the registered index kind.
    #[must_use]
    pub fn kind(&self) -> TypedIndexKind {
        self.index.kind()
    }

    /// Whether the index currently covers every live row of its column.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.drifted_rows == 0
    }

    /// Probe for rows equal to `value`, declining while the index is incomplete.
    #[must_use]
    pub fn lookup_eq(&self, value: &Value) -> Option<Cow<'_, RoaringBitmap>> {
        self.is_complete()
            .then(|| self.index.lookup_eq(value))
            .flatten()
    }

    /// Probe for rows within `range`, declining while the index is incomplete.
    #[must_use]
    pub fn lookup_range<R>(&self, range: R) -> Option<RoaringBitmap>
    where
        R: RangeBounds<Value>,
    {
        self.is_complete()
            .then(|| self.index.lookup_range(range))
            .flatten()
    }

    /// Probe for string rows starting with `prefix`, declining while the index
    /// is incomplete.
    #[must_use]
    pub fn lookup_prefix(&self, prefix: &str) -> Option<RoaringBitmap> {
        self.is_complete()
            .then(|| self.index.lookup_prefix(prefix))
            .flatten()
    }

    /// Probe for rows equal to `value` even when the index is incomplete.
    ///
    /// Reserved for callers auditing the index against its column, which need
    /// to see what the index actually holds rather than what the column says.
    /// `selene.verify` is the intended caller: routing it through the declining
    /// probe would collapse its expected-row count to zero and report every
    /// drifted index as corrupt. Query paths must not use this.
    #[must_use]
    pub fn lookup_eq_ignoring_drift(&self, value: &Value) -> Option<Cow<'_, RoaringBitmap>> {
        self.index.lookup_eq(value)
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
    /// Live rows omitted from `index` because at least one component value's
    /// variant does not match that component's registered kind.
    ///
    /// A composite tuple is all-or-nothing, so one drifted component drops the
    /// whole row. See [`PropertyIndexEntry::drifted_rows`] for why NaN is
    /// excluded.
    pub drifted_rows: u64,
}

impl CompositePropertyIndexEntry {
    /// Construct a composite index entry covering every live row of its columns.
    #[must_use]
    pub fn new(
        index: CompositeTypedIndex,
        declared_properties: SmallVec<[DbString; 4]>,
        name: Option<DbString>,
    ) -> Self {
        Self::new_with_drift(index, declared_properties, name, 0)
    }

    /// Construct a composite index entry carrying a lenient build's drift tally.
    #[must_use]
    pub fn new_with_drift(
        index: CompositeTypedIndex,
        declared_properties: SmallVec<[DbString; 4]>,
        name: Option<DbString>,
        drifted_rows: u64,
    ) -> Self {
        Self {
            index: Arc::new(index),
            declared_properties,
            name,
            drifted_rows,
        }
    }

    /// Return the registered component kinds in declaration order.
    #[must_use]
    pub fn kinds(&self) -> SmallVec<[TypedIndexKind; 4]> {
        self.index.kinds().iter().copied().collect()
    }

    /// Whether the index currently covers every live row of its columns.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.drifted_rows == 0
    }

    /// Hand out the index for probing, declining while it is incomplete.
    ///
    /// The `index` field stays public because `selene.verify` audits the index
    /// against its columns and must see what it actually holds. Query paths go
    /// through here.
    #[must_use]
    pub fn probe_arc(&self) -> Option<Arc<CompositeTypedIndex>> {
        self.is_complete().then(|| Arc::clone(&self.index))
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
