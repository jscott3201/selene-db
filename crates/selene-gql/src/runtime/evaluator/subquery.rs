//! Planned expression-subquery evaluation.
//!
//! `EXISTS { MATCH ... }` follows ISO/IEC 39075:2024 section 19.4 and is
//! two-valued: it returns `TRUE` when the inner pattern has at least one row and
//! `FALSE` otherwise. `COUNT { MATCH ... }` is a selene-db dialect extension
//! over the same planned single-MATCH surface. Correlated outer bindings are
//! projected into the inner pattern's seed row before execution.

use selene_core::Value;

use crate::{
    BindingTableColumn, ExecutionPlan, PatternPlan, PipelineOp, SourceSpan, ValueExpr,
    plan::{OuterBindingRef, PlannedSubquery, SubqueryBody},
    runtime::{
        Binding, BindingTable, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError,
        pattern, plan_runner,
    },
};

pub(super) fn eval_exists(
    expr: &ValueExpr,
    negated: bool,
    _span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let table = execute_subquery(expr, binding, schema, ctx)?;
    let exists = !table.is_empty();
    Ok(Value::Bool(if negated { !exists } else { exists }))
}

pub(super) fn eval_count_subquery(
    expr: &ValueExpr,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let table = execute_subquery(expr, binding, schema, ctx)?;
    let count = i64::try_from(table.row_count()).map_err(|_| {
        ExecutorError::data_exception(
            DataExceptionSubclass::NumericValueOutOfRange,
            "subquery count is out of range",
            span,
        )
    })?;
    Ok(Value::Int(count))
}

pub(super) fn eval_value_subquery(
    expr: &ValueExpr,
    _span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let table = execute_subquery(expr, binding, schema, ctx)?;
    let Some(row) = table.rows().first() else {
        return Ok(Value::Null);
    };
    Ok(row.get(0).cloned().unwrap_or(Value::Null))
}

fn execute_subquery(
    expr: &ValueExpr,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let planned = planned_subquery(expr, ctx)?;
    match &planned.body {
        SubqueryBody::Pattern(plan) => {
            execute_pattern_subquery(planned, plan, binding, schema, ctx)
        }
        SubqueryBody::Plan(plan) => execute_plan_subquery(planned, plan, binding, schema, ctx),
    }
}

fn planned_subquery<'plan>(
    expr: &ValueExpr,
    ctx: &EvalCtx<'_, '_, '_, 'plan>,
) -> Result<&'plan PlannedSubquery, ExecutorError> {
    let expr_id = ctx
        .expr_ids
        .get(expr)
        .ok_or(ExecutorError::ImplementationDefined {
            detail: "subquery expression id missing -- analyzer/lowering bug",
        })?;
    ctx.subqueries
        .get(expr_id)
        .ok_or(ExecutorError::ImplementationDefined {
            detail: "subquery plan missing -- analyzer/lowering bug",
        })
}

fn execute_pattern_subquery(
    planned: &PlannedSubquery,
    plan: &crate::PatternPlan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let target_schema = target_schema_for_pattern(planned, plan, schema)?;
    if null_outer_binding_is_pattern_binding(planned, binding, schema, plan)? {
        return Ok(BindingTable::new(target_schema, Vec::new()));
    }
    let seed = seed_binding(planned, binding, schema, &target_schema)?;
    pattern::execute_pattern_with_seed_and_schema(plan, Some(&seed), target_schema, ctx)
}

fn execute_plan_subquery(
    planned: &PlannedSubquery,
    plan: &ExecutionPlan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let target_schema = target_schema_for_plan(planned, plan, schema)?;
    if null_outer_binding_is_plan_pattern_binding(planned, binding, schema, plan)? {
        return Ok(BindingTable::new(target_schema, Vec::new()));
    }
    let seed = seed_binding(planned, binding, schema, &target_schema)?;
    plan_runner::execute_plan_read_only_with_seed(
        plan,
        Some(BindingTable::new(target_schema, vec![seed])),
        ctx.tx,
    )
}

fn seed_binding(
    planned: &PlannedSubquery,
    row: &Binding,
    source_schema: &BindingTableSchema,
    target_schema: &BindingTableSchema,
) -> Result<Binding, ExecutorError> {
    let mut values = vec![Value::Null; target_schema.columns.len()];
    for outer in &planned.outer_binding_refs {
        let source_index = source_index(source_schema, outer)?;
        let value = row.get(source_index).cloned().unwrap_or(Value::Null);
        let target_index = pattern::column_index(target_schema, outer.name).ok_or(
            ExecutorError::ImplementationDefined {
                detail: "subquery outer binding missing from target row",
            },
        )?;
        values[target_index] = value;
    }
    Ok(Binding::new(values))
}

fn null_outer_binding_is_pattern_binding(
    planned: &PlannedSubquery,
    row: &Binding,
    source_schema: &BindingTableSchema,
    pattern: &PatternPlan,
) -> Result<bool, ExecutorError> {
    for outer in &planned.outer_binding_refs {
        if !pattern_binds_name(pattern, outer.name) {
            continue;
        }
        let source_index = source_index(source_schema, outer)?;
        if matches!(row.get(source_index), Some(Value::Null) | None) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn null_outer_binding_is_plan_pattern_binding(
    planned: &PlannedSubquery,
    row: &Binding,
    source_schema: &BindingTableSchema,
    plan: &ExecutionPlan,
) -> Result<bool, ExecutorError> {
    for outer in &planned.outer_binding_refs {
        if !plan_binds_name(plan, outer.name) {
            continue;
        }
        let source_index = source_index(source_schema, outer)?;
        if matches!(row.get(source_index), Some(Value::Null) | None) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn plan_binds_name(plan: &ExecutionPlan, name: selene_core::IStr) -> bool {
    plan.pattern_plan
        .as_ref()
        .is_some_and(|pattern| pattern_binds_name(pattern, name))
        || plan.pipeline.iter().any(|op| op_binds_name(op, name))
}

fn op_binds_name(op: &PipelineOp, name: selene_core::IStr) -> bool {
    match op {
        PipelineOp::Match(pattern) | PipelineOp::OptionalMatch(pattern) => {
            pattern_binds_name(pattern, name)
        }
        PipelineOp::Union { rhs, .. }
        | PipelineOp::Chain(rhs)
        | PipelineOp::ExplainPlan { inner: rhs, .. } => plan_binds_name(rhs, name),
        PipelineOp::CallSubquery(subquery) => plan_binds_name(&subquery.body, name),
        _ => false,
    }
}

fn pattern_binds_name(pattern: &PatternPlan, name: selene_core::IStr) -> bool {
    pattern.bindings.iter().any(|binding| binding.name == name)
}

fn target_schema_for_pattern(
    planned: &PlannedSubquery,
    plan: &crate::PatternPlan,
    source_schema: &BindingTableSchema,
) -> Result<BindingTableSchema, ExecutorError> {
    let mut schema = pattern::schema_for_pattern(plan);
    append_outer_bindings(&mut schema, &planned.outer_binding_refs, source_schema)?;
    Ok(schema)
}

fn target_schema_for_plan(
    planned: &PlannedSubquery,
    plan: &ExecutionPlan,
    source_schema: &BindingTableSchema,
) -> Result<BindingTableSchema, ExecutorError> {
    let mut schema = plan
        .pattern_plan
        .as_ref()
        .map(pattern::schema_for_pattern)
        .unwrap_or_else(|| BindingTableSchema {
            columns: Vec::new(),
        });
    append_outer_bindings(&mut schema, &planned.outer_binding_refs, source_schema)?;
    Ok(schema)
}

fn append_outer_bindings(
    schema: &mut BindingTableSchema,
    outer_binding_refs: &[OuterBindingRef],
    source_schema: &BindingTableSchema,
) -> Result<(), ExecutorError> {
    for outer in outer_binding_refs {
        if pattern::column_index(schema, outer.name).is_some() {
            continue;
        }
        let source_index = source_index(source_schema, outer)?;
        let source_column = &source_schema.columns[source_index];
        schema.columns.push(BindingTableColumn {
            name: Some(outer.name),
            hidden: None,
            ty: source_column.ty.clone(),
        });
    }
    Ok(())
}

fn source_index(
    source_schema: &BindingTableSchema,
    outer: &OuterBindingRef,
) -> Result<usize, ExecutorError> {
    pattern::column_index(source_schema, outer.name).ok_or(ExecutorError::ImplementationDefined {
        detail: "subquery outer binding missing from source row",
    })
}
