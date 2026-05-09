//! Access-path planner IR.

use std::cmp::Ordering;

use selene_core::IStr;

use crate::{
    Literal, OrderDirection,
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
        /// Typed-index value kind.
        kind: IndexKind,
        /// Lookup bounds.
        bounds: TypedIndexBounds,
    },
    /// Bitmap union over a small set of literal point lookups.
    BitmapUnion {
        /// Opaque catalog handle for the selected typed index.
        handle: IndexHandle,
        /// Literal lookup keys.
        keys: Vec<Literal>,
    },
    /// Composite multi-property exact lookup.
    CompositeLookup {
        /// Opaque catalog handle for the selected composite index.
        handle: IndexHandle,
        /// Indexed properties in declaration order.
        properties: Vec<IStr>,
        /// Literal lookup keys in declaration order.
        keys: Vec<(IStr, Literal)>,
    },
}

/// Bounds for a typed index lookup.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TypedIndexBounds {
    /// Exact equality lookup.
    Equality(Literal),
    /// Exclusive lower bound.
    GreaterThan(Literal),
    /// Inclusive lower bound.
    GreaterEqual(Literal),
    /// Exclusive upper bound.
    LessThan(Literal),
    /// Inclusive upper bound.
    LessEqual(Literal),
    /// Closed or half-open range lookup.
    Range {
        /// Lower bound literal.
        lo: Literal,
        /// Whether the lower bound is inclusive.
        lo_inclusive: bool,
        /// Upper bound literal.
        hi: Literal,
        /// Whether the upper bound is inclusive.
        hi_inclusive: bool,
    },
    /// Multiple exact lookup keys.
    InList(Vec<Literal>),
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
