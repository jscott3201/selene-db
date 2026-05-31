//! Mock optimizer index catalog for integration tests.

use std::collections::HashMap;

use selene_core::{IStr, Value};
use selene_gql::{
    CompositeIndexHandle, IndexCatalog, IndexHandle, IndexKind, IndexTarget, TypedIndexLookup,
};

/// Test-only index catalog with deterministic handles.
///
/// In addition to index discovery, the mock can inject synthetic OPT-5 cost
/// statistics (`with_total_rows`, `with_label_cardinality`,
/// `with_equality_cardinality`, `with_composite_cardinality`,
/// `with_typed_avg_bucket`, `with_composite_avg_bucket`) so cost-gate tests can
/// deterministically flip the chosen plan. Statistics not injected return
/// `None` (the no-stats fallback). `Value` is not `Hash`/`Eq`, so value-keyed
/// statistics are kept as association lists matched by `PartialEq`.
#[derive(Clone, Debug, Default)]
pub struct MockIndexCatalog {
    typed: HashMap<(IndexTarget, IStr, IStr), TypedIndexLookup>,
    labels: HashMap<(IndexTarget, IStr), IndexHandle>,
    composites: HashMap<(IndexTarget, IStr, Vec<IStr>), CompositeIndexHandle>,
    next_handle: u64,
    total_rows: HashMap<IndexTarget, u64>,
    label_cardinality: HashMap<(IndexTarget, IStr), u64>,
    typed_avg_bucket: HashMap<(IndexTarget, IStr, IStr), u64>,
    composite_avg_bucket: HashMap<(IndexTarget, IStr, Vec<IStr>), u64>,
    equality_cardinality: Vec<EqualityStat>,
    composite_cardinality: Vec<CompositeStat>,
}

/// One injected equality-cardinality stat: `((target, label, property, value), rows)`.
type EqualityStat = ((IndexTarget, IStr, IStr, Value), u64);

/// One injected composite-cardinality stat: `((target, label, canonical_keys), rows)`.
type CompositeStat = ((IndexTarget, IStr, Vec<Value>), u64);

impl MockIndexCatalog {
    /// Construct an empty mock catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a node typed-property index.
    #[must_use]
    pub fn with_node_typed_index(mut self, label: IStr, property: IStr, kind: IndexKind) -> Self {
        self.insert_typed_index(IndexTarget::Node, label, property, kind);
        self
    }

    /// Register a node label index.
    #[must_use]
    pub fn with_node_label_index(mut self, label: IStr) -> Self {
        self.insert_label_index(IndexTarget::Node, label);
        self
    }

    /// Register an edge label index.
    #[must_use]
    pub fn with_edge_label_index(mut self, label: IStr) -> Self {
        self.insert_label_index(IndexTarget::Edge, label);
        self
    }

    /// Register a node composite-property index.
    ///
    /// Each `(property, kind)` pair pins the typed-index kind reported through
    /// the catalog so parameter-aware composite probes (BRIEF-154) can perform
    /// per-component plan-time compatibility checks.
    #[must_use]
    pub fn with_node_composite_index(
        mut self,
        label: IStr,
        properties: Vec<(IStr, IndexKind)>,
    ) -> Self {
        self.insert_composite_index(IndexTarget::Node, label, properties);
        self
    }

    fn insert_typed_index(
        &mut self,
        target: IndexTarget,
        label: IStr,
        property: IStr,
        kind: IndexKind,
    ) {
        let handle = self.next_handle();
        self.typed.insert(
            (target, label, property),
            TypedIndexLookup::new(handle, kind),
        );
    }

    fn insert_label_index(&mut self, target: IndexTarget, label: IStr) {
        let handle = self.next_handle();
        self.labels.insert((target, label), handle);
    }

    fn insert_composite_index(
        &mut self,
        target: IndexTarget,
        label: IStr,
        properties: Vec<(IStr, IndexKind)>,
    ) {
        let handle = self.next_handle();
        let mut key_properties: Vec<IStr> =
            properties.iter().map(|(property, _)| *property).collect();
        key_properties.sort();
        self.composites.insert(
            (target, label, key_properties),
            CompositeIndexHandle::new(handle, properties),
        );
    }

    fn next_handle(&mut self) -> IndexHandle {
        self.next_handle = self.next_handle.saturating_add(1);
        IndexHandle::new(self.next_handle)
    }

    // ---- OPT-5 synthetic cost statistics ----

    /// Inject the total live row count for a target (the Linear-scan baseline).
    #[must_use]
    pub fn with_total_rows(mut self, target: IndexTarget, rows: u64) -> Self {
        self.total_rows.insert(target, rows);
        self
    }

    /// Inject the exact row count carrying a node label.
    #[must_use]
    pub fn with_label_cardinality(mut self, label: IStr, rows: u64) -> Self {
        self.label_cardinality
            .insert((IndexTarget::Node, label), rows);
        self
    }

    /// Inject the exact match count for a literal equality probe.
    #[must_use]
    pub fn with_equality_cardinality(
        mut self,
        label: IStr,
        property: IStr,
        value: Value,
        rows: u64,
    ) -> Self {
        self.equality_cardinality
            .push(((IndexTarget::Node, label, property, value), rows));
        self
    }

    /// Inject the average bucket size for a typed index (parameter equality).
    #[must_use]
    pub fn with_typed_avg_bucket(mut self, label: IStr, property: IStr, rows: u64) -> Self {
        self.typed_avg_bucket
            .insert((IndexTarget::Node, label, property), rows);
        self
    }

    /// Inject the exact match count for a literal composite probe. `keys` are in
    /// the index's canonical (sorted-property) order.
    #[must_use]
    pub fn with_composite_cardinality(mut self, label: IStr, keys: Vec<Value>, rows: u64) -> Self {
        self.composite_cardinality
            .push(((IndexTarget::Node, label, keys), rows));
        self
    }

    /// Inject the average bucket size for a composite index (parameter keys).
    /// `properties` are sorted to the canonical key order.
    #[must_use]
    pub fn with_composite_avg_bucket(
        mut self,
        label: IStr,
        mut properties: Vec<IStr>,
        rows: u64,
    ) -> Self {
        properties.sort();
        self.composite_avg_bucket
            .insert((IndexTarget::Node, label, properties), rows);
        self
    }
}

impl IndexCatalog for MockIndexCatalog {
    fn typed_index(
        &self,
        target: IndexTarget,
        label: IStr,
        property: IStr,
    ) -> Option<TypedIndexLookup> {
        self.typed.get(&(target, label, property)).copied()
    }

    fn label_index(&self, target: IndexTarget, label: IStr) -> Option<IndexHandle> {
        self.labels.get(&(target, label)).copied()
    }

    fn composite_index(
        &self,
        target: IndexTarget,
        label: IStr,
        properties: &[IStr],
    ) -> Option<CompositeIndexHandle> {
        let mut key_properties = properties.to_vec();
        key_properties.sort();
        self.composites
            .get(&(target, label, key_properties))
            .cloned()
    }

    fn total_rows(&self, target: IndexTarget) -> Option<u64> {
        self.total_rows.get(&target).copied()
    }

    fn label_cardinality(&self, target: IndexTarget, label: IStr) -> Option<u64> {
        self.label_cardinality.get(&(target, label)).copied()
    }

    fn equality_cardinality(
        &self,
        target: IndexTarget,
        label: IStr,
        property: IStr,
        value: &Value,
    ) -> Option<u64> {
        self.equality_cardinality
            .iter()
            .find(|((t, l, p, v), _)| *t == target && *l == label && *p == property && v == value)
            .map(|(_, rows)| *rows)
    }

    fn typed_avg_bucket(&self, target: IndexTarget, label: IStr, property: IStr) -> Option<u64> {
        self.typed_avg_bucket
            .get(&(target, label, property))
            .copied()
    }

    fn composite_cardinality(
        &self,
        target: IndexTarget,
        label: IStr,
        _properties: &[IStr],
        keys: &[Value],
    ) -> Option<u64> {
        self.composite_cardinality
            .iter()
            .find(|((t, l, k), _)| *t == target && *l == label && k.as_slice() == keys)
            .map(|(_, rows)| *rows)
    }

    fn composite_avg_bucket(
        &self,
        target: IndexTarget,
        label: IStr,
        properties: &[IStr],
    ) -> Option<u64> {
        let mut key = properties.to_vec();
        key.sort();
        self.composite_avg_bucket
            .get(&(target, label, key))
            .copied()
    }
}
