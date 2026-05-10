//! Mutation-pipeline lowering.

use std::collections::HashMap;

use crate::{
    InsertStatement, MatchClause, MutationPipeline, MutationStatement, MutationTerminator,
    NodePattern, PatternElement, RemoveItem, SetItem, SourceSpan, ValueExpr,
    analyze::{
        AnalyzedStatement, BindingDeclKind, BindingId, BindingUseKind, ElementKind,
        MutationWriteSet, WriteKind, WriteSetEntry,
    },
    plan::{
        BindingTableColumn, BindingTableSchema, ExecutionPlan, ImplDefinedCaps, InsertEndpointRef,
        InsertSiteId, MutationOp, PipelineOp, PlannerError, PropertyInit,
    },
};

use super::{expr, match_clause, not_implemented, visible_after_pattern};

/// Lower a mutation pipeline into an execution plan.
pub(crate) fn lower_mutation(
    pipeline: &MutationPipeline,
    analyzed: &AnalyzedStatement,
) -> Result<ExecutionPlan, PlannerError> {
    let write_set = analyzed
        .write_set
        .as_ref()
        .ok_or(PlannerError::WriteSetMissing {
            span: pipeline.span,
        })?;

    let (mut pattern_plan, prefix_filters, mutation_start) =
        lower_read_prefix(&pipeline.statements, analyzed)?;
    let mut visible = visible_after_pattern(pattern_plan.as_ref());
    let mut ops = Vec::new();
    if let Some(pattern) = pattern_plan.as_mut() {
        pattern.filters.extend(prefix_filters);
    } else {
        ops.extend(prefix_filters.into_iter().map(PipelineOp::Filter));
    }

    let mut cursor = 0usize;
    let mut ids = InsertSiteIdAlloc::default();
    for statement in &pipeline.statements[mutation_start..] {
        match statement {
            MutationStatement::Match(_) => {
                return not_implemented(
                    "non-leading MATCH (post-pipeline-boundary pattern)",
                    statement.span(),
                );
            }
            MutationStatement::Filter(value) => {
                ops.push(PipelineOp::Filter(expr::filter_predicate(value, analyzed)?));
            }
            MutationStatement::Insert(insert) => lower_insert(
                insert,
                write_set,
                &mut cursor,
                &mut ids,
                analyzed,
                &mut visible,
                &mut ops,
            )?,
            MutationStatement::Set(items) => {
                lower_set_items(items, write_set, &mut cursor, analyzed, &visible, &mut ops)?;
            }
            MutationStatement::Remove(items) => {
                lower_remove_items(items, write_set, &mut cursor, analyzed, &visible, &mut ops)?;
            }
            MutationStatement::Delete(statement) => {
                for _ in &statement.items {
                    let entry = consume_entry(write_set, &mut cursor, statement.span)?;
                    let WriteKind::DeleteTarget {
                        target,
                        element,
                        mode,
                    } = entry.kind
                    else {
                        return Err(mismatch(entry.span));
                    };
                    let target_column_index =
                        mutation_target_column_index(target, element, analyzed, &visible)?;
                    ops.push(PipelineOp::Mutation(MutationOp::DeleteTarget {
                        target,
                        element,
                        target_column_index,
                        mode,
                        span: entry.span,
                    }));
                }
            }
        }
    }
    if let Some(entry) = write_set.entries.get(cursor) {
        return Err(mismatch(entry.span));
    }

    if let Some(terminator) = &pipeline.terminator {
        match terminator {
            MutationTerminator::Return(clause) => {
                super::lower_return(clause, analyzed, &mut ops, &mut visible)?;
            }
            MutationTerminator::Finish(_) => visible.clear(),
        }
    } else {
        visible.clear();
    }

    let next_pipeline_op_id = crate::PipelineOpId::new(ops.len() as u32);
    Ok(ExecutionPlan {
        category: analyzed.category,
        pattern_plan,
        pipeline: ops,
        output_schema: BindingTableSchema { columns: visible },
        impl_defined_caps: ImplDefinedCaps::default(),
        next_expr_id: super::next_expr_id(analyzed),
        next_pipeline_op_id,
    })
}

fn lower_read_prefix(
    statements: &[MutationStatement],
    analyzed: &AnalyzedStatement,
) -> Result<
    (
        Option<crate::plan::PatternPlan>,
        Vec<crate::plan::FilterPredicate>,
        usize,
    ),
    PlannerError,
> {
    let mut matches: Vec<&MatchClause> = Vec::new();
    let mut filters = Vec::new();
    for (index, statement) in statements.iter().enumerate() {
        match statement {
            MutationStatement::Match(clause) => matches.push(clause),
            MutationStatement::Filter(value) => {
                filters.push(expr::filter_predicate(value, analyzed)?);
            }
            _ => {
                let pattern_plan = match_clause::lower_match_prefix(&matches, analyzed)?;
                return Ok((pattern_plan, filters, index));
            }
        }
    }
    let pattern_plan = match_clause::lower_match_prefix(&matches, analyzed)?;
    Ok((pattern_plan, filters, statements.len()))
}

fn lower_insert(
    insert: &InsertStatement,
    write_set: &MutationWriteSet,
    cursor: &mut usize,
    ids: &mut InsertSiteIdAlloc,
    analyzed: &AnalyzedStatement,
    visible: &mut Vec<BindingTableColumn>,
    ops: &mut Vec<PipelineOp>,
) -> Result<(), PlannerError> {
    for pattern in &insert.patterns {
        let mut sites = HashMap::new();
        for (index, element) in pattern.elements.iter().enumerate() {
            if classify_insert_element(element, analyzed)? == InsertSiteEmission::Emitted {
                sites.insert(index, ids.alloc());
            }
        }

        let mut pending = Vec::new();
        for (index, element) in pattern.elements.iter().enumerate() {
            let Some(site_id) = sites.get(&index).copied() else {
                continue;
            };
            match element {
                PatternElement::Node(node) => {
                    let entry = consume_entry(write_set, cursor, node.span)?;
                    let WriteKind::InsertNode {
                        binding,
                        label_expr,
                        property_keys: _,
                    } = entry.kind
                    else {
                        return Err(mismatch(entry.span));
                    };
                    pending.push(PendingInsert::Node {
                        site_id,
                        binding,
                        label_expr,
                        property_inits: property_inits(&node.properties, analyzed)?,
                        span: node.span,
                    });
                }
                PatternElement::Edge(edge) => {
                    let entry = consume_entry(write_set, cursor, edge.span)?;
                    let WriteKind::InsertEdge {
                        binding,
                        label_expr,
                        property_keys: _,
                    } = entry.kind
                    else {
                        return Err(mismatch(entry.span));
                    };
                    pending.push(PendingInsert::Edge {
                        index,
                        site_id,
                        binding,
                        label_expr,
                        direction: edge.direction,
                        property_inits: property_inits(&edge.properties, analyzed)?,
                        span: edge.span,
                    });
                }
            }
        }

        for item in &pending {
            if let PendingInsert::Node {
                site_id,
                binding,
                label_expr,
                property_inits,
                span,
            } = item
            {
                let (output_column_index, output_column) =
                    push_insert_output(*binding, analyzed, visible)?;
                ops.push(PipelineOp::Mutation(MutationOp::InsertNode {
                    site_id: *site_id,
                    binding: *binding,
                    label_expr: label_expr.clone(),
                    property_inits: property_inits.clone(),
                    output_column_index,
                    output_column: output_column.clone(),
                    span: *span,
                }));
            }
        }
        for item in pending {
            if let PendingInsert::Edge {
                index,
                site_id,
                binding,
                label_expr,
                direction,
                property_inits,
                span,
            } = item
            {
                let left = endpoint_ref(
                    pattern
                        .elements
                        .get(index.wrapping_sub(1))
                        .ok_or_else(|| mismatch(span))?,
                    index.wrapping_sub(1),
                    &sites,
                    analyzed,
                    visible,
                    span,
                )?;
                let right = endpoint_ref(
                    pattern
                        .elements
                        .get(index + 1)
                        .ok_or_else(|| mismatch(span))?,
                    index + 1,
                    &sites,
                    analyzed,
                    visible,
                    span,
                )?;
                let (output_column_index, output_column) =
                    push_insert_output(binding, analyzed, visible)?;
                ops.push(PipelineOp::Mutation(MutationOp::InsertEdge {
                    site_id,
                    binding,
                    label_expr,
                    left,
                    right,
                    direction,
                    property_inits,
                    output_column_index,
                    output_column,
                    span,
                }));
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
enum PendingInsert {
    Node {
        site_id: InsertSiteId,
        binding: Option<BindingId>,
        label_expr: Option<crate::LabelExpr>,
        property_inits: Vec<PropertyInit>,
        span: SourceSpan,
    },
    Edge {
        index: usize,
        site_id: InsertSiteId,
        binding: Option<BindingId>,
        label_expr: Option<crate::LabelExpr>,
        direction: crate::EdgeDirection,
        property_inits: Vec<PropertyInit>,
        span: SourceSpan,
    },
}

fn lower_set_items(
    items: &[SetItem],
    write_set: &MutationWriteSet,
    cursor: &mut usize,
    analyzed: &AnalyzedStatement,
    visible: &[BindingTableColumn],
    ops: &mut Vec<PipelineOp>,
) -> Result<(), PlannerError> {
    for item in items {
        match item {
            SetItem::Property {
                key, value, span, ..
            } => {
                let entry = consume_entry(write_set, cursor, *span)?;
                let WriteKind::SetProperty {
                    target,
                    element,
                    key: entry_key,
                    ..
                } = entry.kind
                else {
                    return Err(mismatch(entry.span));
                };
                if entry_key != *key {
                    return Err(mismatch(entry.span));
                }
                let target_column_index =
                    mutation_target_column_index(target, element, analyzed, visible)?;
                ops.push(PipelineOp::Mutation(MutationOp::SetProperty {
                    target,
                    element,
                    target_column_index,
                    key: *key,
                    value: expr::project_expr(value, None, analyzed)?,
                    span: entry.span,
                }));
            }
            SetItem::PropertyMerge {
                properties, span, ..
            } => {
                for (key, value) in properties {
                    let entry = consume_entry(write_set, cursor, *span)?;
                    let WriteKind::SetProperty {
                        target,
                        element,
                        key: entry_key,
                        ..
                    } = entry.kind
                    else {
                        return Err(mismatch(entry.span));
                    };
                    if entry_key != *key {
                        return Err(mismatch(entry.span));
                    }
                    let target_column_index =
                        mutation_target_column_index(target, element, analyzed, visible)?;
                    ops.push(PipelineOp::Mutation(MutationOp::SetProperty {
                        target,
                        element,
                        target_column_index,
                        key: *key,
                        value: expr::project_expr(value, None, analyzed)?,
                        span: entry.span,
                    }));
                }
            }
            SetItem::Label { label, span, .. } => {
                let entry = consume_entry(write_set, cursor, *span)?;
                let WriteKind::SetLabel {
                    target,
                    element,
                    label: entry_label,
                } = entry.kind
                else {
                    return Err(mismatch(entry.span));
                };
                if entry_label != *label {
                    return Err(mismatch(entry.span));
                }
                let target_column_index =
                    mutation_target_column_index(target, element, analyzed, visible)?;
                ops.push(PipelineOp::Mutation(MutationOp::SetLabel {
                    target,
                    element,
                    target_column_index,
                    label: *label,
                    span: entry.span,
                }));
            }
        }
    }
    Ok(())
}

fn lower_remove_items(
    items: &[RemoveItem],
    write_set: &MutationWriteSet,
    cursor: &mut usize,
    analyzed: &AnalyzedStatement,
    visible: &[BindingTableColumn],
    ops: &mut Vec<PipelineOp>,
) -> Result<(), PlannerError> {
    for item in items {
        match item {
            RemoveItem::Property { key, span, .. } => {
                let entry = consume_entry(write_set, cursor, *span)?;
                let WriteKind::RemoveProperty {
                    target,
                    element,
                    key: entry_key,
                } = entry.kind
                else {
                    return Err(mismatch(entry.span));
                };
                if entry_key != *key {
                    return Err(mismatch(entry.span));
                }
                let target_column_index =
                    mutation_target_column_index(target, element, analyzed, visible)?;
                ops.push(PipelineOp::Mutation(MutationOp::RemoveProperty {
                    target,
                    element,
                    target_column_index,
                    key: *key,
                    span: entry.span,
                }));
            }
            RemoveItem::Label { label, span, .. } => {
                let entry = consume_entry(write_set, cursor, *span)?;
                let WriteKind::RemoveLabel {
                    target,
                    element,
                    label: entry_label,
                } = entry.kind
                else {
                    return Err(mismatch(entry.span));
                };
                if entry_label != *label {
                    return Err(mismatch(entry.span));
                }
                let target_column_index =
                    mutation_target_column_index(target, element, analyzed, visible)?;
                ops.push(PipelineOp::Mutation(MutationOp::RemoveLabel {
                    target,
                    element,
                    target_column_index,
                    label: *label,
                    span: entry.span,
                }));
            }
        }
    }
    Ok(())
}

fn property_inits(
    properties: &[(selene_core::IStr, ValueExpr)],
    analyzed: &AnalyzedStatement,
) -> Result<Vec<PropertyInit>, PlannerError> {
    properties
        .iter()
        .map(|(key, value)| {
            Ok(PropertyInit {
                key: *key,
                value: expr::project_expr(value, None, analyzed)?,
                span: value.span(),
            })
        })
        .collect()
}

fn classify_insert_element(
    element: &PatternElement,
    analyzed: &AnalyzedStatement,
) -> Result<InsertSiteEmission, PlannerError> {
    let (name, span, expected) = match element {
        PatternElement::Node(node) => (node.binding, node.span, BindingDeclKind::InsertNode),
        PatternElement::Edge(edge) => (edge.binding, edge.span, BindingDeclKind::InsertEdge),
    };
    let Some(name) = name else {
        return Ok(InsertSiteEmission::Emitted);
    };
    if fresh_insert_binding(name, span, expected, analyzed).is_some() {
        return Ok(InsertSiteEmission::Emitted);
    }
    if pattern_reuse_binding(name, span, analyzed).is_some() {
        return Ok(InsertSiteEmission::Skipped);
    }
    Err(mismatch(span))
}

fn endpoint_ref(
    element: &PatternElement,
    index: usize,
    sites: &HashMap<usize, InsertSiteId>,
    analyzed: &AnalyzedStatement,
    visible: &[BindingTableColumn],
    span: SourceSpan,
) -> Result<InsertEndpointRef, PlannerError> {
    let PatternElement::Node(node) = element else {
        return Err(mismatch(span));
    };
    if let Some(binding) = named_node_endpoint(node, analyzed)? {
        return Ok(InsertEndpointRef::Binding {
            binding,
            column_index: binding_column_index(binding, analyzed, visible)?,
        });
    }
    sites
        .get(&index)
        .copied()
        .map(InsertEndpointRef::InsertedNode)
        .ok_or_else(|| mismatch(span))
}

fn named_node_endpoint(
    node: &NodePattern,
    analyzed: &AnalyzedStatement,
) -> Result<Option<BindingId>, PlannerError> {
    let Some(name) = node.binding else {
        return Ok(None);
    };
    if let Some(binding) = pattern_reuse_binding(name, node.span, analyzed) {
        return Ok(Some(binding));
    }
    if let Some(binding) =
        fresh_insert_binding(name, node.span, BindingDeclKind::InsertNode, analyzed)
    {
        return Ok(Some(binding));
    }
    Err(mismatch(node.span))
}

fn fresh_insert_binding(
    name: selene_core::IStr,
    span: SourceSpan,
    kind: BindingDeclKind,
    analyzed: &AnalyzedStatement,
) -> Option<BindingId> {
    analyzed
        .scopes
        .declarations()
        .iter()
        .find(|decl| decl.name() == name && decl.span() == span && decl.kind() == kind)
        .map(|decl| decl.id())
}

fn pattern_reuse_binding(
    name: selene_core::IStr,
    span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Option<BindingId> {
    analyzed
        .references
        .iter()
        .find(|reference| {
            reference.name == name
                && reference.span == span
                && reference.kind == BindingUseKind::PatternReuse
        })
        .map(|reference| reference.binding)
}

fn visible_column_for_binding(
    binding: BindingId,
    analyzed: &AnalyzedStatement,
) -> Result<BindingTableColumn, PlannerError> {
    let declaration =
        analyzed
            .scopes
            .declaration(binding)
            .ok_or(PlannerError::BindingResolutionLost {
                binding,
                span: analyzed.span,
            })?;
    Ok(BindingTableColumn {
        name: Some(declaration.name()),
        hidden: None,
        ty: declaration.ty().clone(),
    })
}

fn push_insert_output(
    binding: Option<BindingId>,
    analyzed: &AnalyzedStatement,
    visible: &mut Vec<BindingTableColumn>,
) -> Result<(Option<u32>, Option<BindingTableColumn>), PlannerError> {
    let Some(binding) = binding else {
        return Ok((None, None));
    };
    let index = u32::try_from(visible.len()).expect("binding table column count fits u32");
    let column = visible_column_for_binding(binding, analyzed)?;
    visible.push(column.clone());
    Ok((Some(index), Some(column)))
}

fn binding_column_index(
    binding: BindingId,
    analyzed: &AnalyzedStatement,
    visible: &[BindingTableColumn],
) -> Result<u32, PlannerError> {
    let declaration =
        analyzed
            .scopes
            .declaration(binding)
            .ok_or(PlannerError::BindingResolutionLost {
                binding,
                span: analyzed.span,
            })?;
    let index = visible
        .iter()
        .position(|column| column.name == Some(declaration.name()))
        .ok_or(PlannerError::BindingResolutionLost {
            binding,
            span: declaration.span(),
        })?;
    Ok(u32::try_from(index).expect("binding table column count fits u32"))
}

fn mutation_target_column_index(
    binding: BindingId,
    element: ElementKind,
    analyzed: &AnalyzedStatement,
    visible: &[BindingTableColumn],
) -> Result<u32, PlannerError> {
    match element {
        ElementKind::Node | ElementKind::Edge => binding_column_index(binding, analyzed, visible),
        ElementKind::Path | ElementKind::Alias => Ok(0),
    }
}

fn consume_entry(
    write_set: &MutationWriteSet,
    cursor: &mut usize,
    expected_span: SourceSpan,
) -> Result<WriteSetEntry, PlannerError> {
    let entry = write_set
        .entries
        .get(*cursor)
        .cloned()
        .ok_or_else(|| mismatch(expected_span))?;
    if entry.span != expected_span {
        return Err(mismatch(entry.span));
    }
    *cursor += 1;
    Ok(entry)
}

fn mismatch(span: SourceSpan) -> PlannerError {
    PlannerError::WriteSetPatternMismatch { span }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InsertSiteEmission {
    Emitted,
    Skipped,
}

#[derive(Default)]
struct InsertSiteIdAlloc {
    next: u32,
}

impl InsertSiteIdAlloc {
    fn alloc(&mut self) -> InsertSiteId {
        let id = InsertSiteId::new(self.next);
        self.next += 1;
        id
    }
}

#[cfg(test)]
mod defensive_tests {
    use super::*;
    use crate::{
        EmptyProcedureRegistry, SourceSpan, analyze::StatementCategory, parse, plan::plan,
    };

    #[test]
    fn missing_write_set_reports_planner_error() {
        let analyzed = crate::analyze(
            parse("INSERT (n)").expect("parses"),
            &EmptyProcedureRegistry,
            None,
        )
        .expect("analyzes");
        let mut broken = AnalyzedStatement {
            write_set: None,
            ..analyzed
        };
        broken.category = StatementCategory::DataModifying;
        let err = plan(&broken, &EmptyProcedureRegistry).expect_err("missing write set");
        assert!(matches!(err, PlannerError::WriteSetMissing { .. }));
    }

    #[test]
    fn write_set_shape_mismatch_reports_planner_error() {
        let analyzed = crate::analyze(
            parse("INSERT (n)").expect("parses"),
            &EmptyProcedureRegistry,
            None,
        )
        .expect("analyzes");
        let mut broken = AnalyzedStatement {
            write_set: Some(MutationWriteSet {
                entries: Vec::new(),
            }),
            ..analyzed
        };
        broken.span = SourceSpan::new(0, 10);
        let err = plan(&broken, &EmptyProcedureRegistry).expect_err("mismatch");
        assert!(matches!(err, PlannerError::WriteSetPatternMismatch { .. }));
    }
}
