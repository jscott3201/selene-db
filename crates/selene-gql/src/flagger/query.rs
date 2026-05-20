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
};

use super::{FeatureUse, call, ddl, expr, mutation, record_feature};

pub(crate) fn statement(statement: &Statement, uses: &mut Vec<FeatureUse>) {
    match statement {
        Statement::Query(pipeline) => query_pipeline(pipeline, uses),
        Statement::Composite { first, rest, .. } => {
            query_pipeline(first, uses);
            for (op, pipeline) in rest {
                match op {
                    SetOp::Union | SetOp::UnionAll => {
                        record_feature(uses, FeatureId::GQ03, pipeline.span);
                    }
                    SetOp::Otherwise => record_feature(uses, FeatureId::GQ09, pipeline.span),
                    SetOp::Intersect => record_feature(uses, FeatureId::GQ06, pipeline.span),
                    SetOp::IntersectAll => record_feature(uses, FeatureId::GQ07, pipeline.span),
                    SetOp::Except => record_feature(uses, FeatureId::GQ04, pipeline.span),
                    SetOp::ExceptAll => record_feature(uses, FeatureId::GQ05, pipeline.span),
                }
                query_pipeline(pipeline, uses);
            }
        }
        Statement::Chained { blocks, .. } => {
            for block in blocks {
                query_pipeline(block, uses);
            }
        }
        Statement::Mutate(pipeline) => mutation::pipeline(pipeline, uses),
        Statement::Ddl(statement) => ddl::statement(statement, uses),
        Statement::Call(call) => call::procedure_call(call, uses),
        Statement::Explain { inner, .. } => self::statement(inner, uses),
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. } => record_feature(uses, FeatureId::GT01, statement.span()),
    }
}

pub(crate) fn query_pipeline(pipeline: &QueryPipeline, uses: &mut Vec<FeatureUse>) {
    for statement in &pipeline.statements {
        pipeline_statement(statement, uses);
    }
}

pub(crate) fn pipeline_statement(statement: &PipelineStatement, uses: &mut Vec<FeatureUse>) {
    match statement {
        PipelineStatement::Match(value) => match_clause(value, uses),
        PipelineStatement::Filter(value) => expr::value(value, uses),
        PipelineStatement::Let(values) => let_bindings(values, uses),
        PipelineStatement::Unwind(value) => unwind(value, uses),
        PipelineStatement::Sorting(values) => order_terms(values, uses),
        PipelineStatement::Limit(_) | PipelineStatement::Offset(_) => {}
        PipelineStatement::Return(value) => return_clause(value, uses),
        PipelineStatement::With(value) => with_clause(value, uses),
        PipelineStatement::Call(value) => call::procedure_call(value, uses),
    }
}

pub(crate) fn return_clause(clause: &ReturnClause, uses: &mut Vec<FeatureUse>) {
    if let Some(group_by) = &clause.group_by {
        record_feature(uses, FeatureId::GQ15, clause.span);
        for item in group_by {
            expr::value(item, uses);
        }
    }
    if let Some(having) = &clause.having {
        expr::value(having, uses);
    }
    for item in &clause.items {
        expr::value(&item.expr, uses);
    }
}

pub(crate) fn with_clause(clause: &WithClause, uses: &mut Vec<FeatureUse>) {
    if let Some(group_by) = &clause.group_by {
        record_feature(uses, FeatureId::GQ15, clause.span);
        for item in group_by {
            expr::value(item, uses);
        }
    }
    if let Some(having) = &clause.having {
        expr::value(having, uses);
    }
    if let Some(where_clause) = &clause.where_clause {
        expr::value(where_clause, uses);
    }
    for item in &clause.items {
        expr::value(&item.expr, uses);
    }
}

pub(crate) fn match_clause(clause: &MatchClause, uses: &mut Vec<FeatureUse>) {
    if let Some(selector) = clause.selector {
        match selector {
            PathSelector::All => record_feature(uses, FeatureId::G015, clause.span),
            PathSelector::Any => record_feature(uses, FeatureId::G016, clause.span),
            PathSelector::AllShortest => record_feature(uses, FeatureId::G017, clause.span),
            PathSelector::AnyShortest => record_feature(uses, FeatureId::G018, clause.span),
        }
        // G019/G020 require counted shortest selectors; the current AST has no
        // reachable variant for those forms, so the Flagger cannot emit them yet.
    }
    for pattern in &clause.patterns {
        graph_pattern(pattern, uses);
    }
    if let Some(where_clause) = &clause.where_clause {
        expr::value(where_clause, uses);
    }
}

pub(crate) fn graph_pattern(pattern: &GraphPattern, uses: &mut Vec<FeatureUse>) {
    for element in &pattern.elements {
        match element {
            PatternElement::Node(node) => node_pattern(node, uses),
            PatternElement::Edge(edge) => edge_pattern(edge, uses),
        }
    }
}

fn node_pattern(pattern: &NodePattern, uses: &mut Vec<FeatureUse>) {
    if let Some(label_expr) = &pattern.label_expr {
        label_expression(label_expr);
    }
    for (_, value) in &pattern.properties {
        expr::value(value, uses);
    }
    if let Some(inline_where) = &pattern.inline_where {
        expr::value(inline_where, uses);
    }
}

fn edge_pattern(pattern: &EdgePattern, uses: &mut Vec<FeatureUse>) {
    if let Some(label_expr) = &pattern.label_expr {
        label_expression(label_expr);
    }
    for (_, value) in &pattern.properties {
        expr::value(value, uses);
    }
    if let Some(inline_where) = &pattern.inline_where {
        expr::value(inline_where, uses);
    }
}

fn label_expression(expression: &LabelExpr) {
    match expression {
        LabelExpr::Single(_) | LabelExpr::Wildcard => {}
        LabelExpr::Conjunction(parts) | LabelExpr::Disjunction(parts) => {
            for part in parts {
                label_expression(part);
            }
        }
        LabelExpr::Negation(inner) => label_expression(inner),
    }
}

fn let_bindings(bindings: &[LetBinding], uses: &mut Vec<FeatureUse>) {
    for binding in bindings {
        expr::value(&binding.value, uses);
    }
}

fn unwind(statement: &UnwindStatement, uses: &mut Vec<FeatureUse>) {
    expr::value(&statement.source, uses);
}

fn order_terms(terms: &[OrderTerm], uses: &mut Vec<FeatureUse>) {
    if let Some(first) = terms.first() {
        // Stamp every ORDER BY clause with GA07. The strict spec rule
        // (sort key must be a return alias unless GA07 is claimed) is a
        // bind-pass concern — the Flagger cannot tell at parse time
        // whether a sort key is an alias. The conservative gate is to
        // claim GA07 on any ORDER BY presence; selene-db's v1.0 claim
        // list includes GA07, so this stamp does not produce rejections.
        record_feature(uses, FeatureId::GA07, first.span);
    }
    for term in terms {
        expr::value(&term.expr, uses);
    }
}
