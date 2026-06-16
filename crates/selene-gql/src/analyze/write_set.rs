//! Mutation write-set enumeration.

use selene_core::DbString;

use crate::{
    DeleteMode, LabelExpr, SourceSpan, ValueExpr,
    analyze::binding::{BindingDeclKind, BindingId},
};

/// Enumerated per-pipeline summary of writes produced by a mutation statement.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MutationWriteSet {
    /// Each source write site emits one entry.
    pub entries: Vec<WriteSetEntry>,
}

impl MutationWriteSet {
    pub(crate) fn push(&mut self, entry: WriteSetEntry) {
        self.entries.push(entry);
    }
}

/// One write site in a mutation pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteSetEntry {
    /// Zero-based mutation-statement index within the owning pipeline.
    pub statement_index: usize,
    /// Source span of the write site.
    pub span: SourceSpan,
    /// Kind of write.
    pub kind: WriteKind,
}

/// Structural write operation recorded by the analyzer.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum WriteKind {
    /// `INSERT` introduces a node. Anonymous inserts carry no binding.
    InsertNode {
        /// Inserted-node binding, when the pattern names one.
        binding: Option<BindingId>,
        /// Static label expression declared on the pattern.
        label_expr: Option<LabelExpr>,
        /// Property keys written by the inline property map.
        property_keys: Box<[DbString]>,
    },
    /// `INSERT` introduces an edge. Anonymous inserts carry no binding.
    InsertEdge {
        /// Inserted-edge binding, when the pattern names one.
        binding: Option<BindingId>,
        /// Static label expression declared on the pattern.
        label_expr: Option<LabelExpr>,
        /// Property keys written by the inline property map.
        property_keys: Box<[DbString]>,
    },
    /// `SET n.key = expr` or `SET n = { key: expr }`.
    SetProperty {
        /// Resolved target binding.
        target: BindingId,
        /// Element kind of the resolved target.
        element: ElementKind,
        /// Property key being written.
        key: DbString,
        /// Source span of the value expression for this specific write.
        value_span: SourceSpan,
    },
    /// `SET n :Label`.
    SetLabel {
        /// Resolved target binding.
        target: BindingId,
        /// Element kind of the resolved target.
        element: ElementKind,
        /// Label being added.
        label: DbString,
    },
    /// `REMOVE n.key`.
    RemoveProperty {
        /// Resolved target binding.
        target: BindingId,
        /// Element kind of the resolved target.
        element: ElementKind,
        /// Property key being removed.
        key: DbString,
    },
    /// `REMOVE n :Label`.
    RemoveLabel {
        /// Resolved target binding.
        target: BindingId,
        /// Element kind of the resolved target.
        element: ElementKind,
        /// Label being removed.
        label: DbString,
    },
    /// `DELETE n` / `DETACH DELETE n` / `NODETACH DELETE n`.
    DeleteTarget {
        /// Resolved target binding.
        target: BindingId,
        /// Element kind of the resolved target.
        element: ElementKind,
        /// Delete mode requested by syntax.
        mode: DeleteMode,
    },
}

/// Element kind of a resolved write target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ElementKind {
    /// Node binding.
    Node,
    /// Edge binding.
    Edge,
    /// Path binding.
    Path,
    /// Value alias or procedure output column.
    Alias,
}

impl ElementKind {
    /// Convert a binding declaration kind to its write-target element kind.
    #[must_use]
    pub const fn from_decl_kind(kind: BindingDeclKind) -> Self {
        match kind {
            BindingDeclKind::NodePattern | BindingDeclKind::InsertNode => Self::Node,
            BindingDeclKind::EdgePattern | BindingDeclKind::InsertEdge => Self::Edge,
            BindingDeclKind::PathBinding => Self::Path,
            BindingDeclKind::LetAlias
            | BindingDeclKind::ForAlias
            | BindingDeclKind::ProjectionAlias
            | BindingDeclKind::YieldColumn => Self::Alias,
        }
    }
}

pub(crate) fn property_keys(properties: &[(DbString, ValueExpr)]) -> Box<[DbString]> {
    properties
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>()
        .into_boxed_slice()
}
