//! Analyzed-statement to plan lowering.

mod aggregate;
mod call;
mod catalog;
mod expr;
mod match_clause;
mod mutation;

use crate::{
    LimitValue, PipelineStatement, ProcedureRegistry, QueryPipeline, ReturnClause, SourceSpan,
    WithClause,
    analyze::{AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, ExprId, StatementCategory},
    plan::{
        BindingElement, BindingTableColumn, BindingTableSchema, ExecutionPlan, ImplDefinedCaps,
        LimitAmount, PipelineOp, PlannerError, ProjectExpr, TxOp,
    },
};

/// Lower an analyzed statement into a literal, unoptimized execution plan.
///
/// Dispatches by [`AnalyzedStatementKind`]: queries / set-composed / NEXT-chained
/// pipelines walk the read pipeline; mutations lower from the analyzer's
/// [`MutationWriteSet`]; DDL lowers to a single [`PipelineOp::Catalog`];
/// transaction control lowers to a single [`PipelineOp::Tx`]; top-level CALL
/// looks up procedure metadata in `registry` and lowers to [`PipelineOp::Call`].
///
/// [`MutationWriteSet`]: crate::analyze::MutationWriteSet
pub fn plan(
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
) -> Result<ExecutionPlan, PlannerError> {
    let mut plan = match &analyzed.statement {
        AnalyzedStatementKind::Query(pipeline) => {
            lower_query_pipeline(pipeline, registry, analyzed)
        }
        AnalyzedStatementKind::Composite { first, rest, .. } => {
            let mut plan = lower_query_pipeline(first, registry, analyzed)?;
            for (op, rhs) in rest {
                plan.pipeline.push(PipelineOp::Union {
                    op: *op,
                    rhs: Box::new(lower_query_pipeline(rhs, registry, analyzed)?),
                });
            }
            Ok(plan)
        }
        AnalyzedStatementKind::Chained { blocks, .. } => lower_chained(blocks, registry, analyzed),
        AnalyzedStatementKind::Mutate(pipeline) => mutation::lower_mutation(pipeline, analyzed),
        AnalyzedStatementKind::Ddl(statement) => catalog::lower_ddl(statement, analyzed),
        AnalyzedStatementKind::Call(call) => call::lower_top_level_call(call, registry, analyzed),
        AnalyzedStatementKind::StartTransaction(span) => Ok(tx_plan(TxOp::Start { span: *span })),
        AnalyzedStatementKind::Commit(span) => Ok(tx_plan(TxOp::Commit { span: *span })),
        AnalyzedStatementKind::Rollback(span) => Ok(tx_plan(TxOp::Rollback { span: *span })),
    }?;
    plan.category = analyzed.category;
    plan.refresh_pipeline_op_high_water();
    Ok(plan)
}

fn lower_chained(
    blocks: &[QueryPipeline],
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
) -> Result<ExecutionPlan, PlannerError> {
    let Some((first, rest)) = blocks.split_first() else {
        return Ok(empty_plan());
    };
    let mut plan = lower_query_pipeline(first, registry, analyzed)?;
    // Per BRIEF-26 §O.C6, NEXT establishes a fresh binding scope. The chained
    // plan's output_schema must reflect the final block's projection because
    // each NEXT discards the prior block's columns. Correlated NEXT (rhs
    // references prior-block bindings) is a Phase A scope reduction per
    // BRIEF-35 §O — surface as `PlannerError::NotImplemented` rather than
    // silently lose the carried bindings at runtime.
    for block in rest {
        assert_no_correlated_next(block.span, analyzed)?;
        let inner = lower_query_pipeline(block, registry, analyzed)?;
        plan.output_schema = inner.output_schema.clone();
        plan.pipeline.push(PipelineOp::Chain(Box::new(inner)));
    }
    Ok(plan)
}

fn assert_no_correlated_next(
    block_span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Result<(), PlannerError> {
    for reference in &analyzed.references {
        if !span_contains(block_span, reference.span) {
            continue;
        }
        let Some(decl) = analyzed.scopes.declaration(reference.binding) else {
            continue;
        };
        if !span_contains(block_span, decl.span()) {
            return Err(PlannerError::NotImplemented {
                feature: "correlated NEXT (RHS references prior-block bindings)",
                span: reference.span,
            });
        }
    }
    Ok(())
}

fn span_contains(outer: SourceSpan, inner: SourceSpan) -> bool {
    outer.byte_offset <= inner.byte_offset && inner.end() <= outer.end()
}

fn lower_query_pipeline(
    pipeline: &QueryPipeline,
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
) -> Result<ExecutionPlan, PlannerError> {
    let (matches, tail_start) = leading_matches(&pipeline.statements);
    let pattern_plan = match_clause::lower_match_prefix(&matches, analyzed)?;
    let mut visible = visible_after_pattern(pattern_plan.as_ref());
    let mut ops = Vec::new();
    let tail = &pipeline.statements[tail_start..];
    let mut index = 0;
    while index < tail.len() {
        match &tail[index] {
            PipelineStatement::Match(clause) => {
                return Err(PlannerError::NotImplemented {
                    feature: "non-leading MATCH (post-pipeline-boundary pattern)",
                    span: clause.span,
                });
            }
            PipelineStatement::Filter(value) => {
                ops.push(PipelineOp::Filter(expr::filter_predicate(value, analyzed)?));
            }
            PipelineStatement::Let(bindings) => {
                let projects = bindings
                    .iter()
                    .map(|binding| {
                        expr::project_expr(&binding.value, Some(binding.alias), analyzed)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                for project in &projects {
                    visible.push(BindingTableColumn {
                        name: project.alias,
                        hidden: None,
                        ty: project.ty.clone(),
                    });
                }
                // Why: LET is an extend, not a project — earlier bindings stay
                // visible. PipelineOp::Let preserves them; PipelineOp::Project
                // would drop everything outside the new aliases.
                ops.push(PipelineOp::Let(projects));
            }
            PipelineStatement::Unwind(unwind) => {
                let source = expr::project_expr(&unwind.source, None, analyzed)?;
                visible.push(BindingTableColumn {
                    name: Some(unwind.alias),
                    hidden: None,
                    ty: AnalyzedType::DYNAMIC,
                });
                ops.push(PipelineOp::Unwind {
                    source,
                    alias: unwind.alias,
                    span: unwind.span,
                });
            }
            PipelineStatement::Sorting(terms) => {
                ops.push(PipelineOp::OrderBy(
                    terms
                        .iter()
                        .map(|term| expr::order_key(term, analyzed))
                        .collect::<Result<Vec<_>, _>>()?,
                ));
            }
            PipelineStatement::Offset(offset) => {
                if let Some(PipelineStatement::Limit(limit)) = tail.get(index + 1) {
                    ops.push(PipelineOp::Limit {
                        offset: limit_amount(offset),
                        count: limit_amount(limit),
                    });
                    index += 1;
                } else {
                    ops.push(PipelineOp::Limit {
                        offset: limit_amount(offset),
                        count: LimitAmount::Literal(u64::MAX),
                    });
                }
            }
            PipelineStatement::Limit(limit) => {
                // Why: parsers may emit either `OFFSET .. LIMIT ..` or
                // `LIMIT .. OFFSET ..`. Both forms must collapse to one
                // `Limit { offset, count }` op so executors see "skip then
                // take" semantics regardless of source order.
                if let Some(PipelineStatement::Offset(offset)) = tail.get(index + 1) {
                    ops.push(PipelineOp::Limit {
                        offset: limit_amount(offset),
                        count: limit_amount(limit),
                    });
                    index += 1;
                } else {
                    ops.push(PipelineOp::Limit {
                        offset: LimitAmount::Literal(0),
                        count: limit_amount(limit),
                    });
                }
            }
            PipelineStatement::Return(clause) => {
                lower_return(clause, analyzed, &mut ops, &mut visible)?;
            }
            PipelineStatement::With(clause) => {
                lower_with(clause, analyzed, &mut ops, &mut visible)?;
            }
            PipelineStatement::Call(call) => {
                let planned = call::plan_call(call, registry, analyzed)?;
                visible.extend(call::yield_to_columns(&planned)?);
                ops.push(PipelineOp::Call(planned));
            }
        }
        index += 1;
    }
    let next_pipeline_op_id = crate::PipelineOpId::new(ops.len() as u32);
    Ok(ExecutionPlan {
        category: analyzed.category,
        pattern_plan,
        pipeline: ops,
        output_schema: BindingTableSchema { columns: visible },
        impl_defined_caps: ImplDefinedCaps::default(),
        next_expr_id: next_expr_id(analyzed),
        next_pipeline_op_id,
    })
}

pub(super) fn lower_return(
    clause: &ReturnClause,
    analyzed: &AnalyzedStatement,
    ops: &mut Vec<PipelineOp>,
    visible: &mut Vec<BindingTableColumn>,
) -> Result<(), PlannerError> {
    let aggregate_rewrite = aggregate::push_grouping(
        &clause.group_by,
        &clause.items,
        clause.having.as_ref(),
        analyzed,
        ops,
    )?;
    if let Some(having) = &clause.having {
        ops.push(PipelineOp::Filter(aggregate::filter_predicate(
            having,
            analyzed,
            &aggregate_rewrite.names_by_expr_id,
        )?));
    }
    if clause.star {
        // Why: RETURN * keeps the currently-visible binding table; emitting an
        // empty Project would erase columns that output_schema still advertises.
    } else {
        let projects =
            aggregate::project_items(&clause.items, analyzed, &aggregate_rewrite.names_by_expr_id)?;
        *visible = projects_to_columns(&projects);
        ops.push(PipelineOp::Project(projects));
    }
    if clause.distinct {
        ops.push(PipelineOp::Distinct);
    }
    Ok(())
}

fn lower_with(
    clause: &WithClause,
    analyzed: &AnalyzedStatement,
    ops: &mut Vec<PipelineOp>,
    visible: &mut Vec<BindingTableColumn>,
) -> Result<(), PlannerError> {
    let aggregate_rewrite = aggregate::push_grouping(
        &clause.group_by,
        &clause.items,
        clause.having.as_ref(),
        analyzed,
        ops,
    )?;
    if let Some(having) = &clause.having {
        ops.push(PipelineOp::Filter(aggregate::filter_predicate(
            having,
            analyzed,
            &aggregate_rewrite.names_by_expr_id,
        )?));
    }
    let projects =
        aggregate::project_items(&clause.items, analyzed, &aggregate_rewrite.names_by_expr_id)?;
    *visible = projects_to_columns(&projects);
    ops.push(PipelineOp::Project(projects));
    if clause.distinct {
        ops.push(PipelineOp::Distinct);
    }
    if let Some(where_clause) = &clause.where_clause {
        ops.push(PipelineOp::Filter(expr::filter_predicate(
            where_clause,
            analyzed,
        )?));
    }
    Ok(())
}

fn projects_to_columns(projects: &[ProjectExpr]) -> Vec<BindingTableColumn> {
    projects
        .iter()
        .map(|project| BindingTableColumn {
            name: project.alias,
            hidden: None,
            ty: project.ty.clone(),
        })
        .collect()
}

pub(super) fn visible_after_pattern(
    pattern: Option<&crate::plan::PatternPlan>,
) -> Vec<BindingTableColumn> {
    pattern
        .map(|plan| {
            plan.bindings
                .iter()
                .filter(|binding| binding.element != BindingElement::Path)
                .map(|binding| BindingTableColumn {
                    name: Some(binding.name),
                    hidden: None,
                    ty: binding.ty.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn leading_matches(statements: &[PipelineStatement]) -> (Vec<&crate::MatchClause>, usize) {
    let mut matches = Vec::new();
    for (index, statement) in statements.iter().enumerate() {
        match statement {
            PipelineStatement::Match(clause) => matches.push(clause),
            _ => return (matches, index),
        }
    }
    let len = statements.len();
    (matches, len)
}

fn limit_amount(value: &LimitValue) -> LimitAmount {
    match value {
        LimitValue::Count(value, _) => LimitAmount::Literal(*value),
        LimitValue::Parameter(name, _) => LimitAmount::Parameter(*name),
    }
}

fn empty_plan() -> ExecutionPlan {
    ExecutionPlan {
        category: StatementCategory::ReadOnly,
        pattern_plan: None,
        pipeline: Vec::new(),
        output_schema: BindingTableSchema {
            columns: Vec::new(),
        },
        impl_defined_caps: ImplDefinedCaps::default(),
        next_expr_id: ExprId::new(0),
        next_pipeline_op_id: crate::PipelineOpId::new(0),
    }
}

pub(super) fn not_implemented<T>(
    feature: &'static str,
    span: crate::SourceSpan,
) -> Result<T, PlannerError> {
    Err(PlannerError::NotImplemented { feature, span })
}

fn tx_plan(op: TxOp) -> ExecutionPlan {
    ExecutionPlan {
        category: StatementCategory::TransactionControl,
        pattern_plan: None,
        pipeline: vec![PipelineOp::Tx(op)],
        output_schema: BindingTableSchema {
            columns: Vec::new(),
        },
        impl_defined_caps: ImplDefinedCaps::default(),
        next_expr_id: ExprId::new(0),
        next_pipeline_op_id: crate::PipelineOpId::new(1),
    }
}

pub(super) fn next_expr_id(analyzed: &AnalyzedStatement) -> ExprId {
    ExprId::new(analyzed.expr_types.len() as u32)
}

#[cfg(test)]
mod defensive_tests {
    use super::*;
    use crate::{
        EmptyProcedureRegistry, Literal, ReturnItem, SourceSpan, Statement, ValueExpr,
        analyze::{BindingId, BindingScopeTree, ExprIdMap, ExprTypeTable, StatementCategory},
        parse,
    };

    #[test]
    fn missing_expression_type_reports_planner_error() {
        let expr = ValueExpr::Literal(Literal::Integer(1, SourceSpan::new(7, 1)));
        let statement = AnalyzedStatement {
            statement: AnalyzedStatementKind::Query(QueryPipeline {
                statements: vec![PipelineStatement::Return(ReturnClause {
                    distinct: false,
                    star: false,
                    items: vec![ReturnItem {
                        expr,
                        alias: None,
                        span: SourceSpan::new(7, 1),
                    }],
                    group_by: None,
                    having: None,
                    span: SourceSpan::new(0, 8),
                })],
                span: SourceSpan::new(0, 8),
            }),
            scopes: BindingScopeTree::new(SourceSpan::new(0, 8)),
            references: Vec::new(),
            expr_types: ExprTypeTable::default(),
            expr_ids: ExprIdMap::default(),
            span: SourceSpan::new(0, 8),
            category: StatementCategory::ReadOnly,
            write_set: None,
        };
        let err = plan(&statement, &EmptyProcedureRegistry).expect_err("missing expr cell");
        assert!(matches!(err, PlannerError::ExpressionTypeMissing { .. }));
    }

    #[test]
    fn lost_binding_reference_reports_planner_error() {
        let parsed = parse("RETURN n").expect("test input parses");
        let Statement::Query(parsed_query) = parsed else {
            unreachable!("parser returns query");
        };
        let PipelineStatement::Return(parsed_return) = parsed_query.statements[0].clone() else {
            unreachable!("parser returns return");
        };
        let ValueExpr::Variable { name, .. } = parsed_return.items[0].expr else {
            unreachable!("test projection is variable");
        };
        let mut expr_types = ExprTypeTable::default();
        let expr_id = expr_types.push(crate::AnalyzedType::DYNAMIC);
        let mut statement = AnalyzedStatement {
            statement: AnalyzedStatementKind::Query(QueryPipeline {
                statements: vec![PipelineStatement::Return(parsed_return)],
                span: parsed_query.span,
            }),
            scopes: BindingScopeTree::new(SourceSpan::new(0, 8)),
            references: vec![crate::BindingUse {
                name,
                binding: BindingId::new(999),
                span: SourceSpan::new(7, 1),
                kind: crate::BindingUseKind::Variable,
            }],
            expr_types,
            expr_ids: ExprIdMap::default(),
            span: SourceSpan::new(0, 8),
            category: StatementCategory::ReadOnly,
            write_set: None,
        };
        let AnalyzedStatementKind::Query(query) = &statement.statement else {
            unreachable!("test builds query");
        };
        let PipelineStatement::Return(return_clause) = &query.statements[0] else {
            unreachable!("test builds return");
        };
        let mut expr_ids = ExprIdMap::default();
        expr_ids.insert(&return_clause.items[0].expr, expr_id);
        statement.expr_ids = expr_ids;
        let err = plan(&statement, &EmptyProcedureRegistry).expect_err("lost binding");
        assert!(matches!(err, PlannerError::BindingResolutionLost { .. }));
    }
}
