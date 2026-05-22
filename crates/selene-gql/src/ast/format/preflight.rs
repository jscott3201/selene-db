//! Preflight validation for read-side AST formatting.

use crate::ast::{
    EdgePattern, GqlType, GraphPattern, IsCheckKind, MatchClause, NodePattern, PatternElement,
    ProcedureCall, QueryPipeline, ReturnClause, Statement, ValueExpr, WithClause,
};

use super::FormatError;

/// Validate that a read-side AST can be rendered without semantic loss.
///
/// # Errors
///
/// Returns [`FormatError::Unsupported`] when the tree contains AST-only type
/// variants that the parser cannot read back from formatted source.
pub fn validate_formattable(stmt: &Statement) -> Result<(), FormatError> {
    match stmt {
        Statement::Query(pipeline) => validate_pipeline(pipeline),
        Statement::Composite { first, rest, .. } => {
            validate_pipeline(first)?;
            for (_, pipeline) in rest {
                validate_pipeline(pipeline)?;
            }
            Ok(())
        }
        Statement::Chained { blocks, .. } => {
            for block in blocks {
                validate_pipeline(block)?;
            }
            Ok(())
        }
        Statement::Mutate(_)
        | Statement::Ddl(_)
        | Statement::Call(_)
        | Statement::Explain { .. }
        | Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. } => Ok(()),
    }
}

fn validate_pipeline(pipeline: &QueryPipeline) -> Result<(), FormatError> {
    for statement in &pipeline.statements {
        match statement {
            crate::PipelineStatement::Match(value) => validate_match(value)?,
            crate::PipelineStatement::Filter(value) => validate_expr(value)?,
            crate::PipelineStatement::Let(values) => {
                for value in values {
                    validate_expr(&value.value)?;
                }
            }
            crate::PipelineStatement::Unwind(value) => validate_expr(&value.source)?,
            crate::PipelineStatement::Sorting(values) => {
                for value in values {
                    validate_expr(&value.expr)?;
                }
            }
            crate::PipelineStatement::Limit(_) | crate::PipelineStatement::Offset(_) => {}
            crate::PipelineStatement::Return(value) => validate_return(value)?,
            crate::PipelineStatement::With(value) => validate_with(value)?,
            crate::PipelineStatement::Call(value) => validate_procedure_call(value)?,
        }
    }
    Ok(())
}

fn validate_match(clause: &MatchClause) -> Result<(), FormatError> {
    for pattern in &clause.patterns {
        validate_graph_pattern(pattern)?;
    }
    if let Some(where_clause) = &clause.where_clause {
        validate_expr(where_clause)?;
    }
    Ok(())
}

fn validate_graph_pattern(pattern: &GraphPattern) -> Result<(), FormatError> {
    for element in &pattern.elements {
        match element {
            PatternElement::Node(node) => validate_node_pattern(node)?,
            PatternElement::Edge(edge) => validate_edge_pattern(edge)?,
        }
    }
    Ok(())
}

fn validate_node_pattern(node: &NodePattern) -> Result<(), FormatError> {
    for (_, value) in &node.properties {
        validate_expr(value)?;
    }
    if let Some(where_clause) = &node.inline_where {
        validate_expr(where_clause)?;
    }
    Ok(())
}

fn validate_edge_pattern(edge: &EdgePattern) -> Result<(), FormatError> {
    for (_, value) in &edge.properties {
        validate_expr(value)?;
    }
    if let Some(where_clause) = &edge.inline_where {
        validate_expr(where_clause)?;
    }
    Ok(())
}

fn validate_return(clause: &ReturnClause) -> Result<(), FormatError> {
    for item in &clause.items {
        validate_expr(&item.expr)?;
    }
    if let Some(group_by) = &clause.group_by {
        for item in group_by {
            validate_expr(item)?;
        }
    }
    if let Some(having) = &clause.having {
        validate_expr(having)?;
    }
    Ok(())
}

fn validate_with(clause: &WithClause) -> Result<(), FormatError> {
    for item in &clause.items {
        validate_expr(&item.expr)?;
    }
    if let Some(group_by) = &clause.group_by {
        for item in group_by {
            validate_expr(item)?;
        }
    }
    if let Some(having) = &clause.having {
        validate_expr(having)?;
    }
    if let Some(where_clause) = &clause.where_clause {
        validate_expr(where_clause)?;
    }
    Ok(())
}

pub(super) fn validate_procedure_call(call: &ProcedureCall) -> Result<(), FormatError> {
    for arg in &call.args {
        validate_expr(arg)?;
    }
    Ok(())
}

fn validate_expr(expr: &ValueExpr) -> Result<(), FormatError> {
    match expr {
        ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => Ok(()),
        ValueExpr::PropertyAccess { target, .. } => validate_expr(target),
        ValueExpr::ListAccess { target, index, .. } => {
            validate_expr(target)?;
            validate_expr(index)
        }
        ValueExpr::ListLiteral { items, .. } => validate_exprs(items),
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                validate_expr(value)?;
            }
            Ok(())
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            validate_expr(lhs)?;
            validate_expr(rhs)
        }
        ValueExpr::UnaryOp { operand, .. } => validate_expr(operand),
        ValueExpr::FunctionCall { args, .. } => validate_exprs(args),
        ValueExpr::IsCheck { operand, kind, .. } => {
            validate_expr(operand)?;
            validate_is_check(kind)
        }
        ValueExpr::InList { operand, list, .. } => {
            validate_expr(operand)?;
            validate_exprs(list)
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => {
            validate_expr(operand)?;
            validate_expr(pattern)
        }
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            validate_expr(operand)?;
            validate_expr(low)?;
            validate_expr(high)
        }
        ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => {
            validate_exprs(items)
        }
        ValueExpr::PropertyExists { target, .. } => validate_expr(target),
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (condition, result) in branches {
                validate_expr(condition)?;
                validate_expr(result)?;
            }
            if let Some(value) = else_branch {
                validate_expr(value)?;
            }
            Ok(())
        }
        ValueExpr::Exists { pattern, .. } | ValueExpr::CountSubquery { pattern, .. } => {
            validate_match(pattern)
        }
    }
}

fn validate_exprs(items: &[ValueExpr]) -> Result<(), FormatError> {
    for item in items {
        validate_expr(item)?;
    }
    Ok(())
}

fn validate_is_check(kind: &IsCheckKind) -> Result<(), FormatError> {
    match kind {
        IsCheckKind::Typed(ty) => validate_type(ty),
        IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => validate_expr(value),
        IsCheckKind::Null
        | IsCheckKind::Directed
        | IsCheckKind::Labeled(_)
        | IsCheckKind::TruthValue(_)
        | IsCheckKind::Normalized(_) => Ok(()),
    }
}

fn validate_type(ty: &GqlType) -> Result<(), FormatError> {
    if let Some(variant) = ast_only_type_variant(ty) {
        return Err(FormatError::Unsupported { variant });
    }
    if let GqlType::List(inner) = ty {
        validate_type(inner)?;
    }
    Ok(())
}

fn ast_only_type_variant(ty: &GqlType) -> Option<&'static str> {
    match ty {
        GqlType::Record(_) => Some("Record"),
        GqlType::GraphRef => Some("GraphRef"),
        GqlType::NodeRef => Some("NodeRef"),
        GqlType::EdgeRef => Some("EdgeRef"),
        GqlType::TableRef => Some("TableRef"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::validate_formattable;
    use crate::{
        GqlType, IsCheckKind, Literal, PipelineStatement, QueryPipeline, RecordType, ReturnClause,
        ReturnItem, SourceSpan, Statement, ValueExpr, ast::FormatError,
    };

    #[test]
    fn preflight_rejects_record_type() {
        assert_unsupported(GqlType::Record(RecordType::Open), "Record");
    }

    #[test]
    fn preflight_rejects_graph_ref_type() {
        assert_unsupported(GqlType::GraphRef, "GraphRef");
    }

    #[test]
    fn preflight_rejects_node_ref_type() {
        assert_unsupported(GqlType::NodeRef, "NodeRef");
    }

    #[test]
    fn preflight_rejects_edge_ref_type() {
        assert_unsupported(GqlType::EdgeRef, "EdgeRef");
    }

    #[test]
    fn preflight_rejects_table_ref_type() {
        assert_unsupported(GqlType::TableRef, "TableRef");
    }

    fn assert_unsupported(ty: GqlType, expected: &'static str) {
        let err = validate_formattable(&statement_with_type(ty)).expect_err("type is unsupported");
        match err {
            FormatError::Unsupported { variant } => assert_eq!(variant, expected),
            FormatError::Fmt(_) => panic!("expected unsupported variant"),
        }
    }

    fn statement_with_type(ty: GqlType) -> Statement {
        let span = SourceSpan::default();
        Statement::Query(QueryPipeline {
            statements: vec![PipelineStatement::Return(ReturnClause {
                distinct: false,
                star: false,
                items: vec![ReturnItem {
                    expr: ValueExpr::IsCheck {
                        operand: Box::new(ValueExpr::Literal(Literal::Null(span))),
                        kind: IsCheckKind::Typed(ty),
                        negated: false,
                        span,
                    },
                    alias: None,
                    span,
                }],
                group_by: None,
                having: None,
                span,
            })],
            span,
        })
    }
}
