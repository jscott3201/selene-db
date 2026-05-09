//! Mutation write-set enumeration.

use selene_core::IStr;

use crate::{
    DeleteMode, EdgePattern, GraphPattern, LabelExpr, MutationPipeline, MutationStatement,
    NodePattern, PatternElement, RemoveItem, SetItem, SourceSpan,
    analyze::{
        binding::{BindingDeclKind, BindingId, BindingUse, BindingUseKind},
        scope::BindingScopeTree,
    },
};

/// Enumerated per-pipeline summary of writes produced by a mutation statement.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MutationWriteSet {
    /// Each source write site emits one entry.
    pub entries: Vec<WriteSetEntry>,
}

/// One write site in a mutation pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteSetEntry {
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
        property_keys: Box<[IStr]>,
    },
    /// `INSERT` introduces an edge. Anonymous inserts carry no binding.
    InsertEdge {
        /// Inserted-edge binding, when the pattern names one.
        binding: Option<BindingId>,
        /// Static label expression declared on the pattern.
        label_expr: Option<LabelExpr>,
        /// Property keys written by the inline property map.
        property_keys: Box<[IStr]>,
    },
    /// `SET n.key = expr` or `SET n = { key: expr }`.
    SetProperty {
        /// Resolved target binding.
        target: BindingId,
        /// Element kind of the resolved target.
        element: ElementKind,
        /// Property key being written.
        key: IStr,
    },
    /// `SET n :Label`.
    SetLabel {
        /// Resolved target binding.
        target: BindingId,
        /// Element kind of the resolved target.
        element: ElementKind,
        /// Label being added.
        label: IStr,
    },
    /// `REMOVE n.key`.
    RemoveProperty {
        /// Resolved target binding.
        target: BindingId,
        /// Element kind of the resolved target.
        element: ElementKind,
        /// Property key being removed.
        key: IStr,
    },
    /// `REMOVE n :Label`.
    RemoveLabel {
        /// Resolved target binding.
        target: BindingId,
        /// Element kind of the resolved target.
        element: ElementKind,
        /// Label being removed.
        label: IStr,
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
            | BindingDeclKind::UnwindAlias
            | BindingDeclKind::ProjectionAlias
            | BindingDeclKind::YieldColumn => Self::Alias,
        }
    }
}

pub(crate) fn compute_write_set(
    pipeline: &MutationPipeline,
    scopes: &BindingScopeTree,
    references: &[BindingUse],
) -> MutationWriteSet {
    let mut entries = Vec::new();
    for statement in &pipeline.statements {
        match statement {
            MutationStatement::Match(_) | MutationStatement::Filter(_) => {}
            MutationStatement::Insert(insert) => {
                for pattern in &insert.patterns {
                    collect_insert_pattern(pattern, scopes, references, &mut entries);
                }
            }
            MutationStatement::Set(items) => {
                for item in items {
                    collect_set_item(item, scopes, references, &mut entries);
                }
            }
            MutationStatement::Remove(items) => {
                for item in items {
                    collect_remove_item(item, scopes, references, &mut entries);
                }
            }
            MutationStatement::Delete(statement) => {
                for target in &statement.items {
                    let (binding, element) = resolve_target(
                        *target,
                        statement.span,
                        BindingUseKind::DeleteTarget,
                        scopes,
                        references,
                    );
                    entries.push(WriteSetEntry {
                        span: statement.span,
                        kind: WriteKind::DeleteTarget {
                            target: binding,
                            element,
                            mode: statement.mode,
                        },
                    });
                }
            }
        }
    }
    MutationWriteSet { entries }
}

fn collect_insert_pattern(
    pattern: &GraphPattern,
    scopes: &BindingScopeTree,
    references: &[BindingUse],
    entries: &mut Vec<WriteSetEntry>,
) {
    for element in &pattern.elements {
        match element {
            PatternElement::Node(node) => collect_insert_node(node, scopes, references, entries),
            PatternElement::Edge(edge) => collect_insert_edge(edge, scopes, references, entries),
        }
    }
}

fn collect_insert_node(
    node: &NodePattern,
    scopes: &BindingScopeTree,
    references: &[BindingUse],
    entries: &mut Vec<WriteSetEntry>,
) {
    let Some(binding) = insert_binding(
        node.binding,
        node.span,
        BindingDeclKind::InsertNode,
        scopes,
        references,
    ) else {
        return;
    };
    entries.push(WriteSetEntry {
        span: node.span,
        kind: WriteKind::InsertNode {
            binding,
            label_expr: node.label_expr.clone(),
            property_keys: property_keys(&node.properties),
        },
    });
}

fn collect_insert_edge(
    edge: &EdgePattern,
    scopes: &BindingScopeTree,
    references: &[BindingUse],
    entries: &mut Vec<WriteSetEntry>,
) {
    let Some(binding) = insert_binding(
        edge.binding,
        edge.span,
        BindingDeclKind::InsertEdge,
        scopes,
        references,
    ) else {
        return;
    };
    entries.push(WriteSetEntry {
        span: edge.span,
        kind: WriteKind::InsertEdge {
            binding,
            label_expr: edge.label_expr.clone(),
            property_keys: property_keys(&edge.properties),
        },
    });
}

fn insert_binding(
    name: Option<IStr>,
    span: SourceSpan,
    insert_kind: BindingDeclKind,
    scopes: &BindingScopeTree,
    references: &[BindingUse],
) -> Option<Option<BindingId>> {
    let Some(name) = name else {
        return Some(None);
    };

    if let Some(declaration) = scopes
        .declarations()
        .iter()
        .find(|decl| decl.name() == name && decl.span() == span)
    {
        return (declaration.kind() == insert_kind).then_some(Some(declaration.id()));
    }

    let _reference = references
        .iter()
        .find(|reference| {
            reference.name == name
                && reference.span == span
                && reference.kind == BindingUseKind::PatternReuse
        })
        .expect("bound insert pattern has declaration or pattern-reuse reference");
    None
}

fn collect_set_item(
    item: &SetItem,
    scopes: &BindingScopeTree,
    references: &[BindingUse],
    entries: &mut Vec<WriteSetEntry>,
) {
    match item {
        SetItem::Property {
            target, key, span, ..
        } => {
            let (binding, element) = resolve_target(
                *target,
                *span,
                BindingUseKind::SetTarget,
                scopes,
                references,
            );
            entries.push(WriteSetEntry {
                span: *span,
                kind: WriteKind::SetProperty {
                    target: binding,
                    element,
                    key: *key,
                },
            });
        }
        SetItem::PropertyMerge {
            target,
            properties,
            span,
        } => {
            let (binding, element) = resolve_target(
                *target,
                *span,
                BindingUseKind::SetTarget,
                scopes,
                references,
            );
            for (key, _) in properties {
                entries.push(WriteSetEntry {
                    span: *span,
                    kind: WriteKind::SetProperty {
                        target: binding,
                        element,
                        key: *key,
                    },
                });
            }
        }
        SetItem::Label {
            target,
            label,
            span,
        } => {
            let (binding, element) = resolve_target(
                *target,
                *span,
                BindingUseKind::SetTarget,
                scopes,
                references,
            );
            entries.push(WriteSetEntry {
                span: *span,
                kind: WriteKind::SetLabel {
                    target: binding,
                    element,
                    label: *label,
                },
            });
        }
    }
}

fn collect_remove_item(
    item: &RemoveItem,
    scopes: &BindingScopeTree,
    references: &[BindingUse],
    entries: &mut Vec<WriteSetEntry>,
) {
    match item {
        RemoveItem::Property { target, key, span } => {
            let (binding, element) = resolve_target(
                *target,
                *span,
                BindingUseKind::RemoveTarget,
                scopes,
                references,
            );
            entries.push(WriteSetEntry {
                span: *span,
                kind: WriteKind::RemoveProperty {
                    target: binding,
                    element,
                    key: *key,
                },
            });
        }
        RemoveItem::Label {
            target,
            label,
            span,
        } => {
            let (binding, element) = resolve_target(
                *target,
                *span,
                BindingUseKind::RemoveTarget,
                scopes,
                references,
            );
            entries.push(WriteSetEntry {
                span: *span,
                kind: WriteKind::RemoveLabel {
                    target: binding,
                    element,
                    label: *label,
                },
            });
        }
    }
}

fn resolve_target(
    name: IStr,
    span: SourceSpan,
    kind: BindingUseKind,
    scopes: &BindingScopeTree,
    references: &[BindingUse],
) -> (BindingId, ElementKind) {
    let reference = references
        .iter()
        .find(|reference| {
            reference.name == name && reference.span == span && reference.kind == kind
        })
        .expect("bound mutation target has resolved reference");
    let declaration = scopes
        .declaration(reference.binding)
        .expect("resolved binding has declaration");
    (
        reference.binding,
        ElementKind::from_decl_kind(declaration.kind()),
    )
}

fn property_keys(properties: &[(IStr, crate::ValueExpr)]) -> Box<[IStr]> {
    properties
        .iter()
        .map(|(key, _)| *key)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}
