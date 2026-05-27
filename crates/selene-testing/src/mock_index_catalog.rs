//! Mock optimizer index catalog for integration tests.

use std::collections::HashMap;

use selene_core::IStr;
use selene_gql::{
    CompositeIndexHandle, IndexCatalog, IndexHandle, IndexKind, IndexTarget, TypedIndexLookup,
};

/// Test-only index catalog with deterministic handles.
#[derive(Clone, Debug, Default)]
pub struct MockIndexCatalog {
    typed: HashMap<(IndexTarget, IStr, IStr), TypedIndexLookup>,
    labels: HashMap<(IndexTarget, IStr), IndexHandle>,
    composites: HashMap<(IndexTarget, IStr, Vec<IStr>), CompositeIndexHandle>,
    next_handle: u64,
}

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
}
