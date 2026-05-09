//! Mutation planner IR rows.

use selene_core::IStr;

use crate::{
    DeleteMode, EdgeDirection, LabelExpr, SourceSpan,
    analyze::{BindingId, ElementKind},
};

use super::ProjectExpr;

/// Plan-private identity for one INSERT pattern element.
///
/// Allocated densely per [`crate::ExecutionPlan`] during mutation lowering.
/// Lets insert edges reference freshly inserted anonymous nodes that have no
/// analyzer [`BindingId`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InsertSiteId(u32);

impl InsertSiteId {
    /// Construct an insert-site ID from a dense per-plan ordinal.
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the dense ordinal.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Reference to one endpoint of an inserted edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertEndpointRef {
    /// Endpoint resolves to an existing or freshly named binding.
    Binding(BindingId),
    /// Endpoint resolves to an anonymous inserted node site.
    InsertedNode(InsertSiteId),
}

/// One `(key, value)` initializer in an INSERT pattern's property map.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyInit {
    /// Property key.
    pub key: IStr,
    /// Planned property value expression.
    pub value: ProjectExpr,
    /// Source span of the value expression.
    ///
    /// The AST property map does not carry a span for the `(key: expr)` pair
    /// as a unit, so this tracks the value expression specifically.
    pub span: SourceSpan,
}

/// Mutation operation over the upstream binding table.
///
/// `#[non_exhaustive]` so future write-side surfaces (MERGE, conditional
/// upsert, etc.) can land without breaking downstream matches.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MutationOp {
    /// Insert a node.
    InsertNode {
        /// Plan-private site identity for this inserted node.
        site_id: InsertSiteId,
        /// Analyzer binding, when the inserted node is named.
        binding: Option<BindingId>,
        /// Static label expression from the pattern.
        label_expr: Option<LabelExpr>,
        /// Property initializers from the pattern.
        property_inits: Vec<PropertyInit>,
        /// Source span.
        span: SourceSpan,
    },
    /// Insert an edge.
    InsertEdge {
        /// Plan-private site identity for this inserted edge.
        site_id: InsertSiteId,
        /// Analyzer binding, when the inserted edge is named.
        binding: Option<BindingId>,
        /// Static label expression from the pattern.
        label_expr: Option<LabelExpr>,
        /// Syntactic left endpoint.
        left: InsertEndpointRef,
        /// Syntactic right endpoint.
        right: InsertEndpointRef,
        /// Pattern direction.
        direction: EdgeDirection,
        /// Property initializers from the pattern.
        property_inits: Vec<PropertyInit>,
        /// Source span.
        span: SourceSpan,
    },
    /// Set one property on an existing binding.
    SetProperty {
        /// Target binding.
        target: BindingId,
        /// Target element kind.
        element: ElementKind,
        /// Property key.
        key: IStr,
        /// Planned value expression.
        value: ProjectExpr,
        /// Source span.
        span: SourceSpan,
    },
    /// Add a label to an existing binding.
    SetLabel {
        /// Target binding.
        target: BindingId,
        /// Target element kind.
        element: ElementKind,
        /// Label to add.
        label: IStr,
        /// Source span.
        span: SourceSpan,
    },
    /// Remove one property from an existing binding.
    RemoveProperty {
        /// Target binding.
        target: BindingId,
        /// Target element kind.
        element: ElementKind,
        /// Property key.
        key: IStr,
        /// Source span.
        span: SourceSpan,
    },
    /// Remove a label from an existing binding.
    RemoveLabel {
        /// Target binding.
        target: BindingId,
        /// Target element kind.
        element: ElementKind,
        /// Label to remove.
        label: IStr,
        /// Source span.
        span: SourceSpan,
    },
    /// Delete a node, edge, or path target.
    DeleteTarget {
        /// Target binding.
        target: BindingId,
        /// Target element kind.
        element: ElementKind,
        /// Delete mode.
        mode: DeleteMode,
        /// Source span.
        span: SourceSpan,
    },
}
