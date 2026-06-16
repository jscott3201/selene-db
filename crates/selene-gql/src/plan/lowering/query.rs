//! Query-pipeline lowering.

use crate::{
    GqlType, InlineProcedureCall, LimitValue, PipelineStatement, ProcedureRegistry, QueryPipeline,
    ReturnClause, SourceSpan, WithClause, YieldColumn,
    analyze::{AnalyzedStatement, AnalyzedType},
    plan::{
        BindingElement, BindingTableColumn, BindingTableSchema, ExecutionPlan, ImplDefinedCaps,
        LimitAmount, PipelineOp, PlannedTableSubquery, PlannedTableSubqueryYield, PlannerError,
        ProjectExpr,
    },
};

use super::{aggregate, call, expr, match_clause, sequential_match};

pub(super) fn lower_query_pipeline(
    pipeline: &QueryPipeline,
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
    max_quantifier: u32,
) -> Result<ExecutionPlan, PlannerError> {
    let (matches, tail_start) = leading_matches(&pipeline.statements);
    let pattern_plan = match_clause::lower_match_prefix(&matches, analyzed, max_quantifier)?;
    let mut visible = visible_after_pattern(pattern_plan.as_ref());
    let mut ops = Vec::new();
    let tail = &pipeline.statements[tail_start..];
    let mut index = 0;
    while index < tail.len() {
        match &tail[index] {
            PipelineStatement::Match(clause) => {
                sequential_match::lower(clause, analyzed, &mut ops, &mut visible, max_quantifier)?;
            }
            PipelineStatement::Filter(value) => {
                ops.push(PipelineOp::Filter(expr::filter_predicate(value, analyzed)?));
            }
            PipelineStatement::Let(bindings) => {
                let projects = bindings
                    .iter()
                    .map(|binding| {
                        let mut project = expr::project_expr(
                            &binding.value,
                            Some(binding.alias.clone()),
                            analyzed,
                        )?;
                        if let Some(declared_type) = &binding.declared_type {
                            project.ty = AnalyzedType::Resolved(declared_type.clone());
                            project.declared_type = Some(declared_type.clone());
                        }
                        Ok(project)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                for project in &projects {
                    visible.push(BindingTableColumn {
                        name: project.alias.clone(),
                        hidden: None,
                        ty: project.ty.clone(),
                    });
                }
                // Why: LET is an extend, not a project — earlier bindings stay
                // visible. PipelineOp::Let preserves them; PipelineOp::Project
                // would drop everything outside the new aliases.
                ops.push(PipelineOp::Let(projects));
            }
            PipelineStatement::For(statement) => {
                let source = expr::project_expr(&statement.source, None, analyzed)?;
                visible.push(BindingTableColumn {
                    name: Some(statement.alias.clone()),
                    hidden: None,
                    ty: AnalyzedType::DYNAMIC,
                });
                if let Some(position) = &statement.position {
                    visible.push(BindingTableColumn {
                        name: Some(position.alias.clone()),
                        hidden: None,
                        ty: AnalyzedType::Resolved(GqlType::Integer),
                    });
                }
                ops.push(PipelineOp::Unwind {
                    source,
                    alias: statement.alias.clone(),
                    position: statement.position.clone(),
                    span: statement.span,
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
                visible.extend(planned.yield_schema.clone());
                ops.push(PipelineOp::Call(planned));
            }
            PipelineStatement::CallSubquery(call) => {
                let planned = lower_call_subquery(call, registry, analyzed, max_quantifier)?;
                visible.extend(planned.yield_schema.clone());
                ops.push(PipelineOp::CallSubquery(Box::new(planned)));
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
        expr_ids: analyzed.expr_ids.clone(),
        subqueries: Default::default(),
        next_expr_id: super::next_expr_id(analyzed),
        next_pipeline_op_id,
    })
}

fn lower_call_subquery(
    call: &InlineProcedureCall,
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
    max_quantifier: u32,
) -> Result<PlannedTableSubquery, PlannerError> {
    // GP03: explicit variable scope is bound in the analyzer (the body sees only
    // the named imports); the import set flows into `outer_binding_refs` below
    // via `outer_binding_refs_in_span` because body references resolve to the
    // outer bindings' ids. No planner-side scope handling is required.
    let mut body = lower_query_pipeline(&call.body, registry, analyzed, max_quantifier)?;
    expr::populate_plan_subqueries(&mut body, analyzed, registry, max_quantifier)?;
    let yield_items = table_subquery_yields(call, &body.output_schema)?;
    let yield_schema = yield_items
        .iter()
        .map(|item| {
            let column = body_column(&body.output_schema, item.source.clone(), item.span)?;
            Ok(BindingTableColumn {
                name: Some(item.output.clone()),
                hidden: None,
                ty: nullable_call_yield_type(column.ty.clone(), call.optional),
            })
        })
        .collect::<Result<Vec<_>, PlannerError>>()?;
    Ok(PlannedTableSubquery {
        optional: call.optional,
        body: Box::new(body),
        outer_binding_refs: expr::outer_binding_refs_in_span(call.body.span, analyzed)?,
        yield_items,
        yield_schema,
        span: call.span,
    })
}

fn table_subquery_yields(
    call: &InlineProcedureCall,
    output_schema: &BindingTableSchema,
) -> Result<Vec<PlannedTableSubqueryYield>, PlannerError> {
    let mut yields = Vec::new();
    for item in &call.yield_items {
        match &item.column {
            YieldColumn::Star => {
                for column in output_schema
                    .columns
                    .iter()
                    .filter_map(|column| column.name.clone())
                {
                    yields.push(PlannedTableSubqueryYield {
                        source: column.clone(),
                        output: column,
                        span: item.span,
                    });
                }
            }
            YieldColumn::Named(source) => {
                let output = item.alias.clone().unwrap_or_else(|| source.clone());
                body_column(output_schema, source.clone(), item.span)?;
                yields.push(PlannedTableSubqueryYield {
                    source: source.clone(),
                    output,
                    span: item.span,
                });
            }
        }
    }
    Ok(yields)
}

pub(super) fn nullable_call_yield_type(ty: AnalyzedType, optional: bool) -> AnalyzedType {
    if !optional {
        return ty;
    }
    match ty {
        AnalyzedType::Resolved(ty) => AnalyzedType::Resolved(nullable_gql_type(ty)),
        AnalyzedType::Dynamic => AnalyzedType::Dynamic,
    }
}

fn nullable_gql_type(ty: GqlType) -> GqlType {
    match ty {
        GqlType::NotNull(inner) => *inner,
        other => other,
    }
}

fn body_column(
    schema: &BindingTableSchema,
    name: selene_core::DbString,
    span: SourceSpan,
) -> Result<&BindingTableColumn, PlannerError> {
    schema
        .columns
        .iter()
        .find(|column| column.name == Some(name.clone()))
        .ok_or(PlannerError::ProcedureMetadataMismatch {
            procedure: Box::new([]),
            detail: "CALL subquery yield column missing from body output schema",
            span,
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
            name: project.alias.clone(),
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
                    name: Some(binding.name.clone()),
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
        LimitValue::Parameter {
            name,
            declared_type,
            span,
        } => LimitAmount::Parameter {
            name: name.clone(),
            declared_type: declared_type.clone(),
            span: *span,
        },
    }
}
