//! Query and top-level statement Flagger walk.

use selene_core::feature_register::FeatureId;

use crate::{
    PipelineStatement, QueryPipeline, ReturnClause, SetOp, Statement, WithClause,
    ast::{
        pattern::{
            EdgePattern, GraphPattern, LabelExpr, MatchClause, NodePattern, PathSelector,
            PatternElement,
        },
        statement::{LetBinding, OrderTerm, UnwindStatement},
    },
    error::ParserError,
};

use super::{call, check_feature, ddl, expr, mutation};

pub(crate) fn statement(statement: &Statement) -> Result<(), ParserError> {
    match statement {
        Statement::Query(pipeline) => query_pipeline(pipeline),
        Statement::Composite { first, rest, .. } => {
            query_pipeline(first)?;
            for (op, pipeline) in rest {
                match op {
                    SetOp::Union | SetOp::UnionAll => {
                        check_feature(FeatureId::GQ03, pipeline.span)?
                    }
                    SetOp::Otherwise => check_feature(FeatureId::GQ09, pipeline.span)?,
                    SetOp::Intersect | SetOp::IntersectAll | SetOp::Except | SetOp::ExceptAll => {}
                }
                query_pipeline(pipeline)?;
            }
            Ok(())
        }
        Statement::Chained { blocks, .. } => {
            for block in blocks {
                query_pipeline(block)?;
            }
            Ok(())
        }
        Statement::Mutate(pipeline) => mutation::pipeline(pipeline),
        Statement::Ddl(statement) => ddl::statement(statement),
        Statement::Call(call) => call::procedure_call(call),
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. } => Ok(()),
    }
}

pub(crate) fn query_pipeline(pipeline: &QueryPipeline) -> Result<(), ParserError> {
    for statement in &pipeline.statements {
        pipeline_statement(statement)?;
    }
    Ok(())
}

pub(crate) fn pipeline_statement(statement: &PipelineStatement) -> Result<(), ParserError> {
    match statement {
        PipelineStatement::Match(value) => match_clause(value),
        PipelineStatement::Filter(value) => expr::value(value),
        PipelineStatement::Let(values) => let_bindings(values),
        PipelineStatement::Unwind(value) => unwind(value),
        PipelineStatement::Sorting(values) => order_terms(values),
        PipelineStatement::Limit(_) | PipelineStatement::Offset(_) => Ok(()),
        PipelineStatement::Return(value) => return_clause(value),
        PipelineStatement::With(value) => with_clause(value),
        PipelineStatement::Call(value) => call::procedure_call(value),
    }
}

pub(crate) fn return_clause(clause: &ReturnClause) -> Result<(), ParserError> {
    if let Some(group_by) = &clause.group_by {
        check_feature(FeatureId::GQ15, clause.span)?;
        for item in group_by {
            expr::value(item)?;
        }
    }
    if let Some(having) = &clause.having {
        expr::value(having)?;
    }
    for item in &clause.items {
        expr::value(&item.expr)?;
    }
    Ok(())
}

pub(crate) fn with_clause(clause: &WithClause) -> Result<(), ParserError> {
    if let Some(group_by) = &clause.group_by {
        check_feature(FeatureId::GQ15, clause.span)?;
        for item in group_by {
            expr::value(item)?;
        }
    }
    if let Some(having) = &clause.having {
        expr::value(having)?;
    }
    if let Some(where_clause) = &clause.where_clause {
        expr::value(where_clause)?;
    }
    for item in &clause.items {
        expr::value(&item.expr)?;
    }
    Ok(())
}

pub(crate) fn match_clause(clause: &MatchClause) -> Result<(), ParserError> {
    if let Some(selector) = clause.selector {
        match selector {
            PathSelector::All => check_feature(FeatureId::G015, clause.span)?,
            PathSelector::Any => check_feature(FeatureId::G016, clause.span)?,
            PathSelector::AllShortest => check_feature(FeatureId::G017, clause.span)?,
            PathSelector::AnyShortest => check_feature(FeatureId::G018, clause.span)?,
        }
        // G019/G020 require counted shortest selectors; the current AST has no
        // reachable variant for those forms, so the Flagger cannot emit them yet.
    }
    for pattern in &clause.patterns {
        graph_pattern(pattern)?;
    }
    if let Some(where_clause) = &clause.where_clause {
        expr::value(where_clause)?;
    }
    Ok(())
}

pub(crate) fn graph_pattern(pattern: &GraphPattern) -> Result<(), ParserError> {
    for element in &pattern.elements {
        match element {
            PatternElement::Node(node) => node_pattern(node)?,
            PatternElement::Edge(edge) => edge_pattern(edge)?,
        }
    }
    Ok(())
}

fn node_pattern(pattern: &NodePattern) -> Result<(), ParserError> {
    if let Some(label_expr) = &pattern.label_expr {
        label_expression(label_expr)?;
    }
    for (_, value) in &pattern.properties {
        expr::value(value)?;
    }
    if let Some(inline_where) = &pattern.inline_where {
        expr::value(inline_where)?;
    }
    Ok(())
}

fn edge_pattern(pattern: &EdgePattern) -> Result<(), ParserError> {
    if let Some(label_expr) = &pattern.label_expr {
        label_expression(label_expr)?;
    }
    for (_, value) in &pattern.properties {
        expr::value(value)?;
    }
    if let Some(inline_where) = &pattern.inline_where {
        expr::value(inline_where)?;
    }
    Ok(())
}

fn label_expression(expression: &LabelExpr) -> Result<(), ParserError> {
    match expression {
        LabelExpr::Single(_) | LabelExpr::Wildcard => Ok(()),
        LabelExpr::Conjunction(parts) | LabelExpr::Disjunction(parts) => {
            for part in parts {
                label_expression(part)?;
            }
            Ok(())
        }
        LabelExpr::Negation(inner) => label_expression(inner),
    }
}

fn let_bindings(bindings: &[LetBinding]) -> Result<(), ParserError> {
    for binding in bindings {
        expr::value(&binding.value)?;
    }
    Ok(())
}

fn unwind(statement: &UnwindStatement) -> Result<(), ParserError> {
    expr::value(&statement.source)
}

fn order_terms(terms: &[OrderTerm]) -> Result<(), ParserError> {
    for term in terms {
        expr::value(&term.expr)?;
    }
    Ok(())
}
