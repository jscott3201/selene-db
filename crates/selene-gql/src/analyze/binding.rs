//! Binding identifiers, declarations, and resolved reference records.

use selene_core::IStr;

use crate::{GqlType, LabelExpr, SourceSpan, Vec2OrMore, analyze::types::AnalyzedType};

/// Stable, opaque identifier for a binding declaration.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct BindingId(u32);

impl BindingId {
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return this binding's zero-based numeric index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Type of a binding declaration site.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BindingDeclKind {
    /// A node variable introduced by a `MATCH` node pattern.
    NodePattern,
    /// An edge variable introduced by a `MATCH` edge pattern.
    EdgePattern,
    /// A value alias introduced by `LET`.
    LetAlias,
    /// A value alias introduced by `UNWIND`.
    UnwindAlias,
    /// A projected value introduced by `RETURN` or `WITH`.
    ProjectionAlias,
    /// An explicit result column introduced by `CALL ... YIELD`.
    YieldColumn,
    /// A node variable introduced by an `INSERT` node pattern.
    InsertNode,
    /// An edge variable introduced by an `INSERT` edge pattern.
    InsertEdge,
    /// A path variable introduced by `path = (...)`.
    PathBinding,
}

/// Binding declaration allocated during analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingDecl {
    /// `MATCH (n)`.
    NodePattern {
        /// Allocated binding ID.
        binding: BindingId,
        /// Interned binding name.
        name: IStr,
        /// Source span of the declaration.
        span: SourceSpan,
        /// Static type of the binding.
        ty: AnalyzedType,
        /// Static label predicate written on the pattern, when present.
        labels: Option<LabelExpr>,
    },
    /// `MATCH ()-[e]->()`.
    EdgePattern {
        /// Allocated binding ID.
        binding: BindingId,
        /// Interned binding name.
        name: IStr,
        /// Source span of the declaration.
        span: SourceSpan,
        /// Static type of the binding.
        ty: AnalyzedType,
        /// Static label predicate written on the pattern, when present.
        labels: Option<LabelExpr>,
    },
    /// `LET x = expr`.
    LetAlias {
        /// Allocated binding ID.
        binding: BindingId,
        /// Interned binding name.
        name: IStr,
        /// Source span of the declaration.
        span: SourceSpan,
        /// Static type of the binding.
        ty: AnalyzedType,
    },
    /// `UNWIND expr AS x`.
    UnwindAlias {
        /// Allocated binding ID.
        binding: BindingId,
        /// Interned binding name.
        name: IStr,
        /// Source span of the declaration.
        span: SourceSpan,
        /// Static type of the binding.
        ty: AnalyzedType,
    },
    /// `RETURN expr AS x` or `WITH expr AS x`.
    ProjectionAlias {
        /// Allocated binding ID.
        binding: BindingId,
        /// Interned binding name.
        name: IStr,
        /// Source span of the declaration.
        span: SourceSpan,
        /// Static type of the binding.
        ty: AnalyzedType,
    },
    /// `CALL proc() YIELD x`.
    YieldColumn {
        /// Allocated binding ID.
        binding: BindingId,
        /// Interned binding name.
        name: IStr,
        /// Source span of the declaration.
        span: SourceSpan,
        /// Static type of the binding.
        ty: AnalyzedType,
    },
    /// `INSERT (n)`.
    InsertNode {
        /// Allocated binding ID.
        binding: BindingId,
        /// Interned binding name.
        name: IStr,
        /// Source span of the declaration.
        span: SourceSpan,
        /// Static type of the binding.
        ty: AnalyzedType,
        /// Static label expression written on the inserted node.
        labels: Option<LabelExpr>,
    },
    /// `INSERT ()-[e]->()`.
    InsertEdge {
        /// Allocated binding ID.
        binding: BindingId,
        /// Interned binding name.
        name: IStr,
        /// Source span of the declaration.
        span: SourceSpan,
        /// Static type of the binding.
        ty: AnalyzedType,
        /// Static label expression written on the inserted edge.
        labels: Option<LabelExpr>,
    },
    /// `p = (...)`.
    PathBinding {
        /// Allocated binding ID.
        binding: BindingId,
        /// Interned binding name.
        name: IStr,
        /// Source span of the declaration.
        span: SourceSpan,
        /// Static type of the binding.
        ty: AnalyzedType,
    },
}

impl BindingDecl {
    pub(crate) fn new(
        kind: BindingDeclKind,
        binding: BindingId,
        name: IStr,
        span: SourceSpan,
        ty: AnalyzedType,
        labels: Option<LabelExpr>,
    ) -> Self {
        match kind {
            BindingDeclKind::NodePattern => Self::NodePattern {
                binding,
                name,
                span,
                ty,
                labels,
            },
            BindingDeclKind::EdgePattern => Self::EdgePattern {
                binding,
                name,
                span,
                ty,
                labels,
            },
            BindingDeclKind::LetAlias => Self::LetAlias {
                binding,
                name,
                span,
                ty,
            },
            BindingDeclKind::UnwindAlias => Self::UnwindAlias {
                binding,
                name,
                span,
                ty,
            },
            BindingDeclKind::ProjectionAlias => Self::ProjectionAlias {
                binding,
                name,
                span,
                ty,
            },
            BindingDeclKind::YieldColumn => Self::YieldColumn {
                binding,
                name,
                span,
                ty,
            },
            BindingDeclKind::InsertNode => Self::InsertNode {
                binding,
                name,
                span,
                ty,
                labels,
            },
            BindingDeclKind::InsertEdge => Self::InsertEdge {
                binding,
                name,
                span,
                ty,
                labels,
            },
            BindingDeclKind::PathBinding => Self::PathBinding {
                binding,
                name,
                span,
                ty,
            },
        }
    }

    pub(crate) fn default_type(kind: BindingDeclKind) -> AnalyzedType {
        match kind {
            BindingDeclKind::NodePattern | BindingDeclKind::InsertNode => {
                AnalyzedType::Resolved(GqlType::NodeRef)
            }
            BindingDeclKind::EdgePattern | BindingDeclKind::InsertEdge => {
                AnalyzedType::Resolved(GqlType::EdgeRef)
            }
            BindingDeclKind::PathBinding => AnalyzedType::Resolved(GqlType::Path),
            BindingDeclKind::LetAlias
            | BindingDeclKind::UnwindAlias
            | BindingDeclKind::ProjectionAlias
            | BindingDeclKind::YieldColumn => AnalyzedType::Dynamic,
        }
    }

    /// Return this declaration's allocated binding ID.
    #[must_use]
    pub const fn id(&self) -> BindingId {
        match self {
            Self::NodePattern { binding, .. }
            | Self::EdgePattern { binding, .. }
            | Self::LetAlias { binding, .. }
            | Self::UnwindAlias { binding, .. }
            | Self::ProjectionAlias { binding, .. }
            | Self::YieldColumn { binding, .. }
            | Self::InsertNode { binding, .. }
            | Self::InsertEdge { binding, .. }
            | Self::PathBinding { binding, .. } => *binding,
        }
    }

    /// Return this declaration's interned name.
    #[must_use]
    pub fn name(&self) -> IStr {
        match self {
            Self::NodePattern { name, .. }
            | Self::EdgePattern { name, .. }
            | Self::LetAlias { name, .. }
            | Self::UnwindAlias { name, .. }
            | Self::ProjectionAlias { name, .. }
            | Self::YieldColumn { name, .. }
            | Self::InsertNode { name, .. }
            | Self::InsertEdge { name, .. }
            | Self::PathBinding { name, .. } => name.clone(),
        }
    }

    /// Return this declaration's source span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::NodePattern { span, .. }
            | Self::EdgePattern { span, .. }
            | Self::LetAlias { span, .. }
            | Self::UnwindAlias { span, .. }
            | Self::ProjectionAlias { span, .. }
            | Self::YieldColumn { span, .. }
            | Self::InsertNode { span, .. }
            | Self::InsertEdge { span, .. }
            | Self::PathBinding { span, .. } => *span,
        }
    }

    /// Return this declaration's static type.
    #[must_use]
    pub const fn ty(&self) -> &AnalyzedType {
        match self {
            Self::NodePattern { ty, .. }
            | Self::EdgePattern { ty, .. }
            | Self::LetAlias { ty, .. }
            | Self::UnwindAlias { ty, .. }
            | Self::ProjectionAlias { ty, .. }
            | Self::YieldColumn { ty, .. }
            | Self::InsertNode { ty, .. }
            | Self::InsertEdge { ty, .. }
            | Self::PathBinding { ty, .. } => ty,
        }
    }

    /// Return this declaration's kind.
    #[must_use]
    pub const fn kind(&self) -> BindingDeclKind {
        match self {
            Self::NodePattern { .. } => BindingDeclKind::NodePattern,
            Self::EdgePattern { .. } => BindingDeclKind::EdgePattern,
            Self::LetAlias { .. } => BindingDeclKind::LetAlias,
            Self::UnwindAlias { .. } => BindingDeclKind::UnwindAlias,
            Self::ProjectionAlias { .. } => BindingDeclKind::ProjectionAlias,
            Self::YieldColumn { .. } => BindingDeclKind::YieldColumn,
            Self::InsertNode { .. } => BindingDeclKind::InsertNode,
            Self::InsertEdge { .. } => BindingDeclKind::InsertEdge,
            Self::PathBinding { .. } => BindingDeclKind::PathBinding,
        }
    }

    /// Return the pattern label expression attached to this binding declaration.
    #[must_use]
    pub const fn label_expr(&self) -> Option<&LabelExpr> {
        match self {
            Self::NodePattern { labels, .. }
            | Self::EdgePattern { labels, .. }
            | Self::InsertNode { labels, .. }
            | Self::InsertEdge { labels, .. } => labels.as_ref(),
            Self::LetAlias { .. }
            | Self::UnwindAlias { .. }
            | Self::ProjectionAlias { .. }
            | Self::YieldColumn { .. }
            | Self::PathBinding { .. } => None,
        }
    }

    pub(crate) fn refine_label_expr(&mut self, next: Option<LabelExpr>) {
        let Some(next) = next else {
            return;
        };
        let labels = match self {
            Self::NodePattern { labels, .. }
            | Self::EdgePattern { labels, .. }
            | Self::InsertNode { labels, .. }
            | Self::InsertEdge { labels, .. } => labels,
            Self::LetAlias { .. }
            | Self::UnwindAlias { .. }
            | Self::ProjectionAlias { .. }
            | Self::YieldColumn { .. }
            | Self::PathBinding { .. } => return,
        };
        match labels.take() {
            Some(prior) => {
                *labels = Some(LabelExpr::Conjunction(
                    Vec2OrMore::try_from_vec(vec![prior, next])
                        .expect("refine_label_expr always merges exactly two labels"),
                ));
            }
            None => {
                *labels = Some(next);
            }
        }
    }
}

/// Type of a resolved binding reference.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BindingUseKind {
    /// A normal variable reference in a value expression.
    Variable,
    /// A pattern variable reused after an earlier declaration.
    PatternReuse,
    /// A target variable in `SET`.
    SetTarget,
    /// A target variable in `REMOVE`.
    RemoveTarget,
    /// A target variable in `DELETE`.
    DeleteTarget,
}

/// One resolved reference to a binding declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingUse {
    /// Referenced name.
    pub name: IStr,
    /// Binding declaration resolved by the analyzer.
    pub binding: BindingId,
    /// Source span of the reference.
    pub span: SourceSpan,
    /// Reference category.
    pub kind: BindingUseKind,
}
