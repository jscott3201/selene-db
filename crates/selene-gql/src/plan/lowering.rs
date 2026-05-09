//! Analyzed-statement to plan lowering.

mod expr;
mod match_clause;

use selene_core::IStr;

use crate::{
    LimitValue, PipelineStatement, ProcedureRegistry, QueryPipeline, ReturnClause, ReturnItem,
    ValueExpr, WithClause,
    analyze::{AnalyzedStatement, AnalyzedStatementKind},
    plan::{
        Aggregate, BindingTableColumn, BindingTableSchema, ExecutionPlan, ImplDefinedCaps,
        LimitAmount, PipelineOp, PlannerError,
    },
};

/// Lower an analyzed statement into a literal, unoptimized execution plan.
///
/// BRIEF-26 accepts `registry` to stabilize the M5c public API; CALL lowering
/// returns `NotImplemented` until BRIEF-27 consumes the registry.
pub fn plan(
    analyzed: &AnalyzedStatement,
    _registry: &dyn ProcedureRegistry,
) -> Result<ExecutionPlan, PlannerError> {
    match &analyzed.statement {
        AnalyzedStatementKind::Query(pipeline) => lower_query_pipeline(pipeline, analyzed),
        AnalyzedStatementKind::Composite { first, rest, .. } => {
            let mut plan = lower_query_pipeline(first, analyzed)?;
            for (op, rhs) in rest {
                plan.pipeline.push(PipelineOp::Union {
                    op: *op,
                    rhs: Box::new(lower_query_pipeline(rhs, analyzed)?),
                });
            }
            Ok(plan)
        }
        AnalyzedStatementKind::Chained { blocks, .. } => lower_chained(blocks, analyzed),
        AnalyzedStatementKind::Mutate(_) => not_implemented("Statement::Mutate", analyzed.span),
        AnalyzedStatementKind::Ddl(_) => not_implemented("Statement::Ddl", analyzed.span),
        AnalyzedStatementKind::Call(_) => not_implemented("Statement::Call", analyzed.span),
        AnalyzedStatementKind::StartTransaction(_)
        | AnalyzedStatementKind::Commit(_)
        | AnalyzedStatementKind::Rollback(_) => {
            not_implemented("transaction-control statements", analyzed.span)
        }
    }
}

fn lower_chained(
    blocks: &[QueryPipeline],
    analyzed: &AnalyzedStatement,
) -> Result<ExecutionPlan, PlannerError> {
    let Some((first, rest)) = blocks.split_first() else {
        return Ok(empty_plan());
    };
    let mut plan = lower_query_pipeline(first, analyzed)?;
    for block in rest {
        plan.pipeline
            .push(PipelineOp::Chain(Box::new(lower_query_pipeline(
                block, analyzed,
            )?)));
    }
    Ok(plan)
}

fn lower_query_pipeline(
    pipeline: &QueryPipeline,
    analyzed: &AnalyzedStatement,
) -> Result<ExecutionPlan, PlannerError> {
    let (matches, tail_start) = leading_matches(&pipeline.statements);
    let pattern_plan = match_clause::lower_match_prefix(&matches, analyzed)?;
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
                ops.push(PipelineOp::Project(
                    bindings
                        .iter()
                        .map(|binding| {
                            expr::project_expr(&binding.value, Some(binding.alias), analyzed)
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ));
            }
            PipelineStatement::Unwind(unwind) => {
                ops.push(PipelineOp::Unwind {
                    source: expr::project_expr(&unwind.source, None, analyzed)?,
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
                ops.push(PipelineOp::Limit {
                    offset: LimitAmount::Literal(0),
                    count: limit_amount(limit),
                });
            }
            PipelineStatement::Return(clause) => lower_return(clause, analyzed, &mut ops)?,
            PipelineStatement::With(clause) => lower_with(clause, analyzed, &mut ops)?,
            PipelineStatement::Call(call) => {
                return Err(PlannerError::NotImplemented {
                    feature: "PipelineStatement::Call",
                    span: call.span,
                });
            }
        }
        index += 1;
    }
    Ok(ExecutionPlan {
        pattern_plan,
        pipeline: ops,
        output_schema: output_schema(pipeline, analyzed)?,
        impl_defined_caps: ImplDefinedCaps::default(),
    })
}

fn lower_return(
    clause: &ReturnClause,
    analyzed: &AnalyzedStatement,
    ops: &mut Vec<PipelineOp>,
) -> Result<(), PlannerError> {
    if let Some(keys) = &clause.group_by {
        ops.push(PipelineOp::GroupBy {
            keys: keys
                .iter()
                .map(|value| expr::project_expr(value, column_name(value, None), analyzed))
                .collect::<Result<Vec<_>, _>>()?,
            aggregates: aggregates(&clause.items, analyzed)?,
        });
    }
    if let Some(having) = &clause.having {
        ops.push(PipelineOp::Filter(expr::filter_predicate(
            having, analyzed,
        )?));
    }
    ops.push(PipelineOp::Project(project_items(&clause.items, analyzed)?));
    if clause.distinct {
        ops.push(PipelineOp::Distinct);
    }
    Ok(())
}

fn lower_with(
    clause: &WithClause,
    analyzed: &AnalyzedStatement,
    ops: &mut Vec<PipelineOp>,
) -> Result<(), PlannerError> {
    if let Some(keys) = &clause.group_by {
        ops.push(PipelineOp::GroupBy {
            keys: keys
                .iter()
                .map(|value| expr::project_expr(value, column_name(value, None), analyzed))
                .collect::<Result<Vec<_>, _>>()?,
            aggregates: aggregates(&clause.items, analyzed)?,
        });
    }
    if let Some(having) = &clause.having {
        ops.push(PipelineOp::Filter(expr::filter_predicate(
            having, analyzed,
        )?));
    }
    ops.push(PipelineOp::Project(project_items(&clause.items, analyzed)?));
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

fn project_items(
    items: &[ReturnItem],
    analyzed: &AnalyzedStatement,
) -> Result<Vec<crate::ProjectExpr>, PlannerError> {
    items
        .iter()
        .map(|item| expr::project_expr(&item.expr, column_name(&item.expr, item.alias), analyzed))
        .collect()
}

fn aggregates(
    items: &[ReturnItem],
    analyzed: &AnalyzedStatement,
) -> Result<Vec<Aggregate>, PlannerError> {
    items
        .iter()
        .filter_map(|item| {
            expr::aggregate_name(&item.expr).map(|(function, star, distinct)| {
                let args = match &item.expr {
                    ValueExpr::FunctionCall { args, .. } => args,
                    _ => unreachable!("aggregate_name only matches function calls"),
                };
                Ok(Aggregate {
                    function,
                    args: args
                        .iter()
                        .map(|arg| expr::aggregate_arg(arg, analyzed))
                        .collect::<Result<Vec<_>, _>>()?,
                    star,
                    distinct,
                    span: item.span,
                })
            })
        })
        .collect()
}

fn output_schema(
    pipeline: &QueryPipeline,
    analyzed: &AnalyzedStatement,
) -> Result<BindingTableSchema, PlannerError> {
    for statement in pipeline.statements.iter().rev() {
        match statement {
            PipelineStatement::Return(clause) if clause.star => {
                return Ok(visible_schema(analyzed));
            }
            PipelineStatement::Return(clause) => return projection_schema(&clause.items, analyzed),
            PipelineStatement::With(clause) => return projection_schema(&clause.items, analyzed),
            _ => {}
        }
    }
    Ok(visible_schema(analyzed))
}

fn visible_schema(analyzed: &AnalyzedStatement) -> BindingTableSchema {
    BindingTableSchema {
        columns: analyzed
            .scopes
            .declarations()
            .iter()
            .map(|decl| BindingTableColumn {
                name: Some(decl.name()),
                ty: decl.ty().clone(),
            })
            .collect(),
    }
}

fn projection_schema(
    items: &[ReturnItem],
    analyzed: &AnalyzedStatement,
) -> Result<BindingTableSchema, PlannerError> {
    let mut columns = Vec::new();
    for item in items {
        let (_, ty) = expr::expr_cell(&item.expr, analyzed)?;
        columns.push(BindingTableColumn {
            name: column_name(&item.expr, item.alias),
            ty,
        });
    }
    Ok(BindingTableSchema { columns })
}

fn column_name(expr: &ValueExpr, alias: Option<IStr>) -> Option<IStr> {
    alias.or(match expr {
        ValueExpr::Variable { name, .. } => Some(*name),
        _ => None,
    })
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
        pattern_plan: None,
        pipeline: Vec::new(),
        output_schema: BindingTableSchema {
            columns: Vec::new(),
        },
        impl_defined_caps: ImplDefinedCaps::default(),
    }
}

fn not_implemented<T>(feature: &'static str, span: crate::SourceSpan) -> Result<T, PlannerError> {
    Err(PlannerError::NotImplemented { feature, span })
}

#[cfg(test)]
mod defensive_tests {
    use super::*;
    use crate::{
        EmptyProcedureRegistry, Literal, SourceSpan, Statement,
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
