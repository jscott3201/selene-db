//! Graph-pattern bind handling.

use crate::{
    EdgePattern, GraphPattern, LabelExpr, MatchClause, NodePattern, PatternElement,
    analyze::{
        binding::BindingDeclKind,
        error::{AnalysisError, ConditionClause},
        write_set::{WriteKind, property_keys},
    },
};

use super::{BindContext, expr};

pub(crate) fn bind_match_clause(
    ctx: &mut BindContext,
    clause: &MatchClause,
) -> Result<(), AnalysisError> {
    for pattern in &clause.patterns {
        bind_graph_pattern(ctx, pattern, PatternBindingMode::Match)?;
    }
    if let Some(where_clause) = &clause.where_clause {
        expr::bind_condition(ctx, where_clause, ConditionClause::MatchWhere)?;
    }
    Ok(())
}

pub(crate) fn bind_insert_graph_pattern(
    ctx: &mut BindContext,
    statement_index: usize,
    pattern: &GraphPattern,
) -> Result<(), AnalysisError> {
    bind_graph_pattern(ctx, pattern, PatternBindingMode::Insert { statement_index })
}

fn bind_graph_pattern(
    ctx: &mut BindContext,
    pattern: &GraphPattern,
    mode: PatternBindingMode,
) -> Result<(), AnalysisError> {
    if let Some(name) = pattern.path_binding {
        ctx.declare_or_reuse(BindingDeclKind::PathBinding, name, pattern.span)?;
    }

    for element in &pattern.elements {
        match element {
            PatternElement::Node(node) => bind_node_pattern(ctx, node, mode)?,
            PatternElement::Edge(edge) => bind_edge_pattern(ctx, edge, mode)?,
        }
    }
    Ok(())
}

fn bind_node_pattern(
    ctx: &mut BindContext,
    node: &NodePattern,
    mode: PatternBindingMode,
) -> Result<(), AnalysisError> {
    if let Some(name) = node.binding {
        let kind = match mode {
            PatternBindingMode::Match => BindingDeclKind::NodePattern,
            PatternBindingMode::Insert { .. } => BindingDeclKind::InsertNode,
        };
        let (binding, reused) =
            ctx.declare_or_reuse_with_labels_info(kind, name, node.span, node.label_expr.clone())?;
        if let PatternBindingMode::Insert { statement_index } = mode
            && !reused
        {
            ctx.record_write(
                statement_index,
                node.span,
                WriteKind::InsertNode {
                    binding: Some(binding),
                    label_expr: node.label_expr.clone(),
                    property_keys: property_keys(&node.properties),
                },
            );
        }
    } else if let PatternBindingMode::Insert { statement_index } = mode {
        ctx.record_write(
            statement_index,
            node.span,
            WriteKind::InsertNode {
                binding: None,
                label_expr: node.label_expr.clone(),
                property_keys: property_keys(&node.properties),
            },
        );
    }
    if let Some(label) = &node.label_expr {
        bind_label_expr(label);
    }
    for (_, value) in &node.properties {
        expr::bind_value_expr(ctx, value)?;
    }
    if let Some(where_clause) = &node.inline_where {
        expr::bind_condition(ctx, where_clause, ConditionClause::InlineWhere)?;
    }
    Ok(())
}

fn bind_edge_pattern(
    ctx: &mut BindContext,
    edge: &EdgePattern,
    mode: PatternBindingMode,
) -> Result<(), AnalysisError> {
    if let Some(name) = edge.binding {
        let kind = match mode {
            PatternBindingMode::Match => BindingDeclKind::EdgePattern,
            PatternBindingMode::Insert { .. } => BindingDeclKind::InsertEdge,
        };
        let (binding, reused) =
            ctx.declare_or_reuse_with_labels_info(kind, name, edge.span, edge.label_expr.clone())?;
        if let PatternBindingMode::Insert { statement_index } = mode
            && !reused
        {
            ctx.record_write(
                statement_index,
                edge.span,
                WriteKind::InsertEdge {
                    binding: Some(binding),
                    label_expr: edge.label_expr.clone(),
                    property_keys: property_keys(&edge.properties),
                },
            );
        }
    } else if let PatternBindingMode::Insert { statement_index } = mode {
        ctx.record_write(
            statement_index,
            edge.span,
            WriteKind::InsertEdge {
                binding: None,
                label_expr: edge.label_expr.clone(),
                property_keys: property_keys(&edge.properties),
            },
        );
    }
    if let Some(label) = &edge.label_expr {
        bind_label_expr(label);
    }
    for (_, value) in &edge.properties {
        expr::bind_value_expr(ctx, value)?;
    }
    if let Some(where_clause) = &edge.inline_where {
        expr::bind_condition(ctx, where_clause, ConditionClause::InlineWhere)?;
    }
    Ok(())
}

fn bind_label_expr(label: &LabelExpr) {
    match label {
        LabelExpr::Single(_) | LabelExpr::Wildcard => {}
        LabelExpr::Conjunction(values) | LabelExpr::Disjunction(values) => {
            for value in values {
                bind_label_expr(value);
            }
        }
        LabelExpr::Negation(value) => bind_label_expr(value),
    }
}

#[derive(Clone, Copy)]
enum PatternBindingMode {
    Match,
    Insert { statement_index: usize },
}
