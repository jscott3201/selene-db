//! Access-path planner IR.

use std::cmp::Ordering;

use selene_core::IStr;

use crate::{
    GqlType, Literal, OrderDirection, SourceSpan,
    analyze::BindingId,
    plan::optimize::{IndexHandle, IndexKind},
};

/// Scan access path selected by optimizer rules.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum ScanAccess {
    /// Default: enumerate rows linearly and evaluate predicates row-by-row.
    #[default]
    Linear,
    /// Use a label bitmap to enumerate candidate rows.
    LabelIndex {
        /// Opaque catalog handle for the selected label index.
        handle: IndexHandle,
    },
    /// Use a typed property index for equality or range lookup.
    TypedIndexRange {
        /// Opaque catalog handle for the selected typed index.
        handle: IndexHandle,
        /// Indexed property key.
        property: IStr,
        /// Typed-index value kind.
        kind: IndexKind,
        /// Lookup bounds.
        bounds: TypedIndexBounds,
    },
    /// Bitmap union over a small set of literal-or-parameter point lookups.
    BitmapUnion {
        /// Opaque catalog handle for the selected typed index.
        handle: IndexHandle,
        /// Indexed property key.
        property: IStr,
        /// Typed-index value kind. Carried so runtime parameter resolution
        /// can run the IndexKind-mismatch loud error path against bound values.
        kind: IndexKind,
        /// Lookup keys; each is either an inline literal or a parameter slot.
        keys: Vec<IndexKey>,
    },
    /// Composite multi-property exact lookup.
    CompositeLookup {
        /// Opaque catalog handle for the selected composite index.
        handle: IndexHandle,
        /// Indexed properties in declaration order paired with kinds. The
        /// per-component IndexKind feeds runtime parameter resolution
        /// (BRIEF-154 §B.3 + F17) — Commit 1 already widened
        /// `CompositeIndexHandle.properties`, this carries the same shape
        /// into the executable plan IR.
        properties: Vec<(IStr, IndexKind)>,
        /// Lookup keys in declaration order; each is literal-or-parameter.
        keys: Vec<(IStr, IndexKey)>,
    },
}

/// A single index probe key.
///
/// Literals are pinned at plan time; parameters resolve to a [`selene_core::Value`]
/// against the bound [`crate::runtime::TxContext`] parameters at probe time.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum IndexKey {
    /// Inline literal value pinned at plan time.
    Literal(Literal),
    /// Parameter slot resolved at execute time.
    Parameter {
        /// Parameter name (e.g. `$symbol` → `IStr("symbol")`).
        name: IStr,
        /// Optional declared parameter type, per BRIEF-137 `$id :: TYPE`.
        ///
        /// Plan-time typed-incompatibility checks consult this; the runtime
        /// resolver also validates the declared type against it before the
        /// [`IndexKind`] check.
        declared_type: Option<GqlType>,
        /// Source span for diagnostics.
        span: SourceSpan,
    },
}

/// Bounds for a typed index lookup.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TypedIndexBounds {
    /// Exact equality lookup.
    Equality(IndexKey),
    /// Exclusive lower bound.
    GreaterThan(IndexKey),
    /// Inclusive lower bound.
    GreaterEqual(IndexKey),
    /// Exclusive upper bound.
    LessThan(IndexKey),
    /// Inclusive upper bound.
    LessEqual(IndexKey),
    /// Closed or half-open range lookup.
    Range {
        /// Lower bound key.
        lo: IndexKey,
        /// Whether the lower bound is inclusive.
        lo_inclusive: bool,
        /// Upper bound key.
        hi: IndexKey,
        /// Whether the upper bound is inclusive.
        hi_inclusive: bool,
    },
}

/// Sort-key access hint selected by optimizer rules.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum OrderAccess {
    /// A typed property index can supply non-null indexed rows in this order.
    TypedIndex {
        /// Opaque catalog handle for the selected typed index.
        handle: IndexHandle,
        /// Direction requested for the access path.
        direction: OrderDirection,
    },
}

/// Structural node-id ordering used by WCO symmetry breaking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NodeIdOrdering {
    /// Left-hand node binding.
    pub left: BindingId,
    /// Ordering relation between the two node IDs.
    pub ordering: Ordering,
    /// Right-hand node binding.
    pub right: BindingId,
}
