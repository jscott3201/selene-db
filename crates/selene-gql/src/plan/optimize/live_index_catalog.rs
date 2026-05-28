//! SeleneGraph-backed [`IndexCatalog`] for production optimizer index discovery.
//!
//! [`LiveIndexCatalog`] adapts a *pinned* immutable graph snapshot
//! (`Arc<SeleneGraph>`) to the optimizer's [`IndexCatalog`] trait so the default
//! structural rules can select label / typed / composite index access paths
//! against the live graph. It is the production counterpart to the test-only
//! `MockIndexCatalog`.
//!
//! # Pinned-snapshot invariant (load-bearing)
//!
//! The catalog captures exactly one `Arc<SeleneGraph>` at construction and
//! probes *that* snapshot for every method call — it never re-reads the
//! `ArcSwap`. The optimizer MUST plan against the same snapshot the executor
//! runs against; re-reading per probe could let `optimize()` observe a
//! different index set per call under concurrent index DDL, producing a plan
//! whose access paths don't match the executed snapshot. Embedders build the
//! catalog from the snapshot they pin for execution (see
//! `Session::execute_source`).
//!
//! Because the snapshot is an immutable `ArcSwap` load (D10 lock-free read),
//! constructing and probing the catalog takes no write lock and preserves the
//! single-writer / lock-free-reader contract.
//!
//! # Always-available label index
//!
//! [`label_index`](LiveIndexCatalog::label_index) returns `Some` for any
//! `(Node | Edge, label)` because SeleneGraph maintains the intrinsic
//! RoaringBitmap label index on every write — a label scan is always a valid
//! access path. The runtime arm (`runtime::scan::label_index_rows`) falls back
//! to a linear scan when the requested label's bitmap is absent (zero matching
//! rows), so reporting `Some` here is always safe.

use std::sync::Arc;

use selene_core::IStr;
use selene_graph::{SeleneGraph, TypedIndexKind};

use crate::plan::optimize::{
    CompositeIndexHandle, IndexCatalog, IndexHandle, IndexKind, IndexTarget, TypedIndexLookup,
};

/// Index catalog backed by a pinned [`SeleneGraph`] snapshot.
///
/// See the module docs for the pinned-snapshot invariant and the
/// always-available label-index contract.
#[derive(Clone, Debug)]
pub struct LiveIndexCatalog {
    snapshot: Arc<SeleneGraph>,
}

impl LiveIndexCatalog {
    /// Construct a catalog over a pinned graph snapshot.
    ///
    /// The supplied `Arc<SeleneGraph>` must be the same snapshot the plan will
    /// execute against (see the module-level pinned-snapshot invariant).
    #[must_use]
    pub const fn new(snapshot: Arc<SeleneGraph>) -> Self {
        Self { snapshot }
    }
}

impl IndexCatalog for LiveIndexCatalog {
    fn typed_index(
        &self,
        target: IndexTarget,
        label: IStr,
        property: IStr,
    ) -> Option<TypedIndexLookup> {
        // Built-in typed property indexes are node-only at HEAD.
        if target != IndexTarget::Node {
            return None;
        }
        let kind = self.snapshot.property_index_for(&label, &property)?.kind();
        // The opaque handle is never dereferenced by the runtime; the scan
        // re-derives the actual index by (label, property) at execute time.
        Some(TypedIndexLookup::new(
            IndexHandle::new(0),
            index_kind_from(kind),
        ))
    }

    fn label_index(&self, target: IndexTarget, _label: IStr) -> Option<IndexHandle> {
        // The intrinsic RoaringBitmap label index is always available for both
        // node and edge targets; the runtime falls back to linear when a label
        // bitmap is absent. The handle is opaque (runtime re-derives by label).
        match target {
            IndexTarget::Node | IndexTarget::Edge => Some(IndexHandle::new(0)),
        }
    }

    fn composite_index(
        &self,
        target: IndexTarget,
        label: IStr,
        properties: &[IStr],
    ) -> Option<CompositeIndexHandle> {
        // Composite indexes are node-only at HEAD.
        if target != IndexTarget::Node {
            return None;
        }
        let mut canonical = properties.to_vec();
        canonical.sort_unstable();
        let entry = self
            .snapshot
            .composite_property_index_entry_for(&label, &canonical)?;
        let kinds = entry.kinds();
        // Per-component IndexKind in declaration order enables parameter-aware
        // composite probes (BRIEF-154 §B.2). The runtime re-derives the actual
        // index by (label, sorted-properties) at execute time.
        let component_kinds: Vec<(IStr, IndexKind)> = entry
            .declared_properties
            .iter()
            .zip(kinds.iter())
            .map(|(property, kind)| (*property, index_kind_from(*kind)))
            .collect();
        Some(CompositeIndexHandle::new(
            IndexHandle::new(0),
            component_kinds,
        ))
    }
}

/// Map a storage-level [`TypedIndexKind`] to the optimizer's [`IndexKind`].
fn index_kind_from(kind: TypedIndexKind) -> IndexKind {
    match kind {
        TypedIndexKind::I64 => IndexKind::Integer,
        TypedIndexKind::F64 => IndexKind::Float,
        TypedIndexKind::String => IndexKind::String,
        TypedIndexKind::Date => IndexKind::Date,
        TypedIndexKind::LocalDateTime => IndexKind::LocalDateTime,
        TypedIndexKind::Uuid => IndexKind::Uuid,
    }
}
