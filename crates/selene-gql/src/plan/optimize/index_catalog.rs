//! Query-time index discovery surface.

use selene_core::IStr;

/// Graph element kind targeted by an index lookup.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum IndexTarget {
    /// Node index lookup.
    Node,
    /// Edge index lookup.
    Edge,
}

/// Embedder-injected catalog for optimizer index discovery.
///
/// Handles are valid only for the catalog snapshot used to optimize a plan.
/// Embedders that cache plans across snapshot rotations must either re-plan or
/// validate handles against the new snapshot before execution.
pub trait IndexCatalog: Send + Sync {
    /// Return a typed-property index for `(target, label, property)`, if any.
    fn typed_index(
        &self,
        target: IndexTarget,
        label: IStr,
        property: IStr,
    ) -> Option<TypedIndexLookup>;

    /// Return a label-only index for `(target, label)`, if any.
    fn label_index(&self, target: IndexTarget, label: IStr) -> Option<IndexHandle>;

    /// Return a composite-property index for `(target, label, properties)`, if any.
    fn composite_index(
        &self,
        target: IndexTarget,
        label: IStr,
        properties: &[IStr],
    ) -> Option<CompositeIndexHandle>;
}

/// Opaque index handle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct IndexHandle(u64);

impl IndexHandle {
    /// Construct an opaque handle from a caller-owned raw value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the caller-owned raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Typed-index lookup metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TypedIndexLookup {
    /// Opaque catalog handle.
    pub handle: IndexHandle,
    /// Indexed value kind.
    pub kind: IndexKind,
}

impl TypedIndexLookup {
    /// Construct typed-index lookup metadata.
    #[must_use]
    pub const fn new(handle: IndexHandle, kind: IndexKind) -> Self {
        Self { handle, kind }
    }
}

/// Composite-index lookup metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CompositeIndexHandle {
    /// Opaque catalog handle.
    pub handle: IndexHandle,
    /// Indexed properties in declaration order, paired with their typed-index
    /// kinds. The kinds enable per-component plan-time compatibility checks
    /// when admitting parameter slots into composite-index probes
    /// (BRIEF-154 §B.2 F7/F17 folds).
    pub properties: Vec<(IStr, IndexKind)>,
}

impl CompositeIndexHandle {
    /// Construct composite-index lookup metadata.
    #[must_use]
    pub fn new(handle: IndexHandle, properties: Vec<(IStr, IndexKind)>) -> Self {
        Self { handle, properties }
    }

    /// Return the property keys in declaration order, dropping the kind column.
    #[must_use]
    pub fn property_keys(&self) -> Vec<IStr> {
        self.properties.iter().map(|(key, _)| *key).collect()
    }
}

/// Indexable value kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum IndexKind {
    /// Signed integer typed index.
    Integer,
    /// Floating-point typed index.
    Float,
    /// String typed index.
    String,
    /// Date typed index.
    Date,
    /// Local datetime typed index.
    LocalDateTime,
    /// UUID typed index.
    Uuid,
}

/// Empty catalog with no registered indexes.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyIndexCatalog;

impl IndexCatalog for EmptyIndexCatalog {
    fn typed_index(
        &self,
        _target: IndexTarget,
        _label: IStr,
        _property: IStr,
    ) -> Option<TypedIndexLookup> {
        None
    }

    fn label_index(&self, _target: IndexTarget, _label: IStr) -> Option<IndexHandle> {
        None
    }

    fn composite_index(
        &self,
        _target: IndexTarget,
        _label: IStr,
        _properties: &[IStr],
    ) -> Option<CompositeIndexHandle> {
        None
    }
}
