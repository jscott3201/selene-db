//! Analyzed-statement to plan lowering.

mod aggregate;
mod binding_refs;
mod call;
mod catalog;
mod expr;
mod match_clause;
mod match_mode;
mod mutation;
mod path_mode;
mod path_search;
mod repeat;
mod sequential_match;
mod set_op;

use set_op::assert_arms_column_name_equal;

use crate::{
    GqlType, InlineProcedureCall, LimitValue, PipelineStatement, ProcedureRegistry, QueryPipeline,
    ReturnClause, SourceSpan, WithClause, YieldColumn,
    analyze::{AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, ExprId, StatementCategory},
    plan::{
        BindingElement, BindingTableColumn, BindingTableSchema, ExecutionPlan, ImplDefinedCaps,
        LimitAmount, PipelineOp, PlannedTableSubquery, PlannedTableSubqueryYield, PlannerError,
        ProjectExpr, SessionOp, TxOp,
    },
};

/// Lower an analyzed statement into a literal, unoptimized execution plan
/// using the default implementation-defined caps ([`ImplDefinedCaps::DEFAULT`]).
///
/// Thin wrapper over [`plan_with_caps`]; call that to inject caller-configured
/// caps (see [`crate::runtime::Session::with_impl_defined_caps`]).
pub fn plan(
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
) -> Result<ExecutionPlan, PlannerError> {
    plan_with_caps(analyzed, registry, &ImplDefinedCaps::DEFAULT)
}

/// Lower an analyzed statement into a literal, unoptimized execution plan,
/// stamping the caller-supplied implementation-defined caps.
///
/// Dispatches by [`AnalyzedStatementKind`]: queries / set-composed / NEXT-chained
/// pipelines walk the read pipeline; mutations lower from the analyzer's
/// [`MutationWriteSet`]; DDL lowers to a single [`PipelineOp::Catalog`];
/// transaction control lowers to a single [`PipelineOp::Tx`]; top-level CALL
/// looks up procedure metadata in `registry` and lowers to [`PipelineOp::Call`].
///
/// `caps` reach the plan two ways: the plan-time variable-length quantifier gate
/// consults `caps.max_quantifier` *during* lowering (threaded as `max_quantifier`),
/// and the finished top-level plan carries `*caps` in `impl_defined_caps` for the
/// runtime context and optimizer. Nested plans (set-op / NEXT / CALL-subquery
/// bodies) execute under the parent context, so only the top-level stamp is read.
///
/// [`MutationWriteSet`]: crate::analyze::MutationWriteSet
#[tracing::instrument(
    name = "selene.gql.plan",
    skip(analyzed, registry, caps),
    fields(category = ?analyzed.category)
)]
pub fn plan_with_caps(
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    caps: &ImplDefinedCaps,
) -> Result<ExecutionPlan, PlannerError> {
    let mut plan = lower_statement_kind(&analyzed.statement, registry, analyzed, caps)?;
    plan.category = analyzed.category;
    plan.expr_ids = analyzed.expr_ids.clone();
    expr::populate_plan_subqueries(&mut plan, analyzed, registry, caps.max_quantifier)?;
    // Why: only the top-level plan's caps are consumed — statement.rs builds the
    // runtime TxContext and the optimizer's OptimizeContext from
    // `plan.impl_defined_caps`, and nested plans execute under that same context.
    // The one cap needed mid-lowering (the quantifier gate) is threaded above as
    // `max_quantifier`, so a post-lowering stamp here covers every statement kind.
    plan.impl_defined_caps = *caps;
    plan.refresh_pipeline_op_high_water();
    Ok(plan)
}

fn lower_statement_kind(
    statement: &AnalyzedStatementKind,
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
    caps: &ImplDefinedCaps,
) -> Result<ExecutionPlan, PlannerError> {
    // The quantifier gate is the only cap threaded recursively mid-lowering; the
    // DDL key-label-set IL003 gate needs the full caps, so `lower_ddl` receives
    // `caps` directly.
    let max_quantifier = caps.max_quantifier;
    match statement {
        AnalyzedStatementKind::Query(pipeline) => {
            lower_query_pipeline(pipeline, registry, analyzed, max_quantifier)
        }
        AnalyzedStatementKind::Composite { first, rest, .. } => {
            let mut plan = lower_query_pipeline(first, registry, analyzed, max_quantifier)?;
            for (op, rhs) in rest {
                let rhs_plan = lower_query_pipeline(rhs, registry, analyzed, max_quantifier)?;
                // ISO §14.2 SR v: set-composition arms must be column
                // name-equal. The binder binds each arm independently, so the
                // names are first available here on the lowered output schemas.
                assert_arms_column_name_equal(
                    *op,
                    &plan.output_schema,
                    &rhs_plan.output_schema,
                    rhs.span,
                )?;
                plan.pipeline.push(PipelineOp::Union {
                    op: *op,
                    rhs: Box::new(rhs_plan),
                });
            }
            Ok(plan)
        }
        AnalyzedStatementKind::Chained { blocks, .. } => {
            lower_chained(blocks, registry, analyzed, max_quantifier)
        }
        AnalyzedStatementKind::Mutate(pipeline) => {
            mutation::lower_mutation(pipeline, analyzed, max_quantifier)
        }
        AnalyzedStatementKind::Ddl(statement) => catalog::lower_ddl(statement, analyzed, caps),
        AnalyzedStatementKind::Call(call) => call::lower_top_level_call(call, registry, analyzed),
        AnalyzedStatementKind::Explain { inner, span } => {
            lower_explain(inner, *span, registry, analyzed, caps)
        }
        AnalyzedStatementKind::StartTransaction(span) => Ok(tx_plan(TxOp::Start { span: *span })),
        AnalyzedStatementKind::Commit(span) => Ok(tx_plan(TxOp::Commit { span: *span })),
        AnalyzedStatementKind::Rollback(span) => Ok(tx_plan(TxOp::Rollback { span: *span })),
        AnalyzedStatementKind::SessionSetValue {
            param,
            value,
            if_not_exists,
            span,
        } => Ok(session_plan(SessionOp::SetValue {
            param: param.clone(),
            value: value.clone(),
            if_not_exists: *if_not_exists,
            span: *span,
        })),
        AnalyzedStatementKind::SessionSetTimeZone { zone, span } => {
            Ok(session_plan(SessionOp::SetTimeZone {
                zone: zone.clone(),
                span: *span,
            }))
        }
        AnalyzedStatementKind::SessionReset { target, span } => {
            Ok(session_plan(session_reset_op(target, *span)))
        }
        AnalyzedStatementKind::SessionClose(span) => {
            Ok(session_plan(SessionOp::Close { span: *span }))
        }
    }
}

fn session_reset_op(target: &crate::SessionResetTarget, span: SourceSpan) -> SessionOp {
    use crate::SessionResetTarget;
    match target {
        SessionResetTarget::AllCharacteristics => SessionOp::ResetAllCharacteristics { span },
        SessionResetTarget::Parameters => SessionOp::ResetParameters { span },
        SessionResetTarget::TimeZone => SessionOp::ResetTimeZone { span },
        SessionResetTarget::Parameter(param) => SessionOp::ResetParameter {
            param: param.clone(),
            span,
        },
    }
}

fn lower_explain(
    inner: &AnalyzedStatementKind,
    span: SourceSpan,
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
    caps: &ImplDefinedCaps,
) -> Result<ExecutionPlan, PlannerError> {
    let inner = lower_statement_kind(inner, registry, analyzed, caps)?;
    Ok(ExecutionPlan {
        category: StatementCategory::ReadOnly,
        pattern_plan: None,
        pipeline: vec![PipelineOp::ExplainPlan {
            inner: Box::new(inner),
            span,
        }],
        output_schema: explain_output_schema(span)?,
        impl_defined_caps: ImplDefinedCaps::default(),
        expr_ids: analyzed.expr_ids.clone(),
        subqueries: Default::default(),
        next_expr_id: next_expr_id(analyzed),
        next_pipeline_op_id: crate::PipelineOpId::new(1),
    })
}

fn lower_chained(
    blocks: &[QueryPipeline],
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
    max_quantifier: u32,
) -> Result<ExecutionPlan, PlannerError> {
    let Some((first, rest)) = blocks.split_first() else {
        return Ok(empty_plan());
    };
    let mut plan = lower_query_pipeline(first, registry, analyzed, max_quantifier)?;
    // NEXT's output_schema must reflect the final block's projection because
    // each NEXT discards the prior block's columns. Correlated NEXT (rhs
    // references prior-block bindings) is rejected here rather than silently
    // losing carried bindings at runtime.
    for block in rest {
        assert_no_correlated_next(block.span, analyzed)?;
        let inner = lower_query_pipeline(block, registry, analyzed, max_quantifier)?;
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
                        expr::project_expr(&binding.value, Some(binding.alias.clone()), analyzed)
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
            PipelineStatement::Unwind(unwind) => {
                let source = expr::project_expr(&unwind.source, None, analyzed)?;
                visible.push(BindingTableColumn {
                    name: Some(unwind.alias.clone()),
                    hidden: None,
                    ty: AnalyzedType::DYNAMIC,
                });
                if let Some(position) = &unwind.position {
                    visible.push(BindingTableColumn {
                        name: Some(position.alias.clone()),
                        hidden: None,
                        ty: AnalyzedType::Resolved(GqlType::Integer),
                    });
                }
                ops.push(PipelineOp::Unwind {
                    source,
                    alias: unwind.alias.clone(),
                    position: unwind.position.clone(),
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
                visible.extend(planned.yield_schema.clone());
                ops.push(PipelineOp::Call(planned));
                if let Some(filter) = &call.yield_filter {
                    ops.push(PipelineOp::Filter(expr::filter_predicate(
                        filter, analyzed,
                    )?));
                }
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
        next_expr_id: next_expr_id(analyzed),
        next_pipeline_op_id,
    })
}

fn lower_call_subquery(
    call: &InlineProcedureCall,
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
    max_quantifier: u32,
) -> Result<PlannedTableSubquery, PlannerError> {
    if call.in_transactions {
        return Err(PlannerError::NotImplemented {
            feature: "CALL { ... } IN TRANSACTIONS",
            span: call.span,
        });
    }
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
                ty: column.ty.clone(),
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

fn explain_output_schema(span: SourceSpan) -> Result<BindingTableSchema, PlannerError> {
    let name = selene_core::db_string("plan").map_err(|_err| {
        PlannerError::StaticStringConstructionFailed {
            detail: "static EXPLAIN column 'plan'",
            span,
        }
    })?;
    Ok(BindingTableSchema {
        columns: vec![BindingTableColumn {
            name: Some(name),
            hidden: None,
            ty: AnalyzedType::Resolved(GqlType::String),
        }],
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

fn empty_plan() -> ExecutionPlan {
    ExecutionPlan {
        category: StatementCategory::ReadOnly,
        pattern_plan: None,
        pipeline: Vec::new(),
        output_schema: BindingTableSchema {
            columns: Vec::new(),
        },
        impl_defined_caps: ImplDefinedCaps::default(),
        expr_ids: Default::default(),
        subqueries: Default::default(),
        next_expr_id: ExprId::new(0),
        next_pipeline_op_id: crate::PipelineOpId::new(0),
    }
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
        expr_ids: Default::default(),
        subqueries: Default::default(),
        next_expr_id: ExprId::new(0),
        next_pipeline_op_id: crate::PipelineOpId::new(1),
    }
}

fn session_plan(op: SessionOp) -> ExecutionPlan {
    ExecutionPlan {
        category: StatementCategory::SessionControl,
        pattern_plan: None,
        pipeline: vec![PipelineOp::Session(op)],
        output_schema: BindingTableSchema {
            columns: Vec::new(),
        },
        impl_defined_caps: ImplDefinedCaps::default(),
        expr_ids: Default::default(),
        subqueries: Default::default(),
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
        analyze::{BindingId, BindingScopeTree, ExprIdLookup, ExprTypeTable, StatementCategory},
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
            expr_ids: ExprIdLookup::default(),
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
        let ValueExpr::Variable { name, .. } = &parsed_return.items[0].expr else {
            unreachable!("test projection is variable");
        };
        let name = name.clone();
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
            expr_ids: ExprIdLookup::default(),
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
        let mut expr_ids = ExprIdLookup::default();
        expr_ids.insert(&return_clause.items[0].expr, expr_id);
        statement.expr_ids = expr_ids;
        let err = plan(&statement, &EmptyProcedureRegistry).expect_err("lost binding");
        assert!(matches!(err, PlannerError::BindingResolutionLost { .. }));
    }
}
