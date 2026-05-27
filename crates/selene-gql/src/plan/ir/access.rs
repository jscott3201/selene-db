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
        /// can run the BRIEF-154 §B.3 F4 ExternalString carve-out and the
        /// F12 IndexKind-mismatch loud error path against bound values.
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
        /// resolver also runs [`crate::runtime::parameter_type::validate_declared_type`]
        /// against it before the [`IndexKind`] check.
        declared_type: Option<GqlType>,
        /// Source span for diagnostics.
        span: SourceSpan,
    },
}

impl IndexKey {
    /// Borrow the inner literal, panicking on parameter slots.
    ///
    /// Bridges runtime sites that have not yet been rewired through
    /// [`crate::runtime::scan::resolve_index_key`] (Commit 4). The optimizer
    /// rules never emit [`IndexKey::Parameter`] until Commit 2, so until then
    /// `Parameter` is genuinely unreachable on the runtime path. Callers MUST
    /// be migrated to the parameter-aware resolver before the rules start
    /// emitting parameter slots; the `unreachable!` is a guardrail, not a
    /// supported pattern.
    #[must_use]
    pub fn literal_for_pre_param_path(&self) -> &Literal {
        match self {
            Self::Literal(literal) => literal,
            Self::Parameter { name, .. } => {
                unreachable!(
                    "IndexKey::Parameter ${} reached a runtime site that has not been \
                     rewired through resolve_index_key; this site must be migrated to \
                     parameter-aware resolution before the optimizer emits Parameter slots",
                    name.as_str(),
                )
            }
        }
    }
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
