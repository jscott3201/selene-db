//! Planned expression-subquery evaluation.
//!
//! `EXISTS { MATCH ... }` follows ISO/IEC 39075:2024 section 19.4 and is
//! two-valued: it returns `TRUE` when the inner pattern has at least one row and
//! `FALSE` otherwise. `COUNT { MATCH ... }` is a selene-db dialect extension
//! over the same planned single-MATCH surface. Correlated outer bindings are
//! projected into the inner pattern's seed row before execution.

use selene_core::Value;

use crate::{
    SourceSpan, ValueExpr,
    plan::PlannedSubquery,
    runtime::{Binding, BindingTable, BindingTableSchema, EvalCtx, ExecutorError, pattern},
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
    let count = i64::try_from(table.row_count()).map_err(|_| ExecutorError::DataException {
        message: "subquery count is out of range".to_owned(),
        span,
    })?;
    Ok(Value::Int(count))
}

fn execute_subquery(
    expr: &ValueExpr,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let planned = planned_subquery(expr, ctx)?;
    let target_schema = pattern::schema_for_pattern(&planned.plan);
    let seed = seed_binding(planned, binding, schema, &target_schema)?;
    pattern::execute_pattern_with_seed(&planned.plan, Some(&seed), ctx)
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

fn seed_binding(
    planned: &PlannedSubquery,
    row: &Binding,
    source_schema: &BindingTableSchema,
    target_schema: &BindingTableSchema,
) -> Result<Binding, ExecutorError> {
    let mut values = vec![Value::Null; target_schema.columns.len()];
    for binding_id in &planned.outer_binding_refs {
        let binding = planned
            .plan
            .bindings
            .iter()
            .find(|candidate| candidate.binding == *binding_id)
            .ok_or(ExecutorError::ImplementationDefined {
                detail: "subquery outer binding missing from pattern plan",
            })?;
        let source_index = source_schema
            .columns
            .iter()
            .position(|column| column.name == Some(binding.name))
            .ok_or(ExecutorError::ImplementationDefined {
                detail: "subquery outer binding missing from source row",
            })?;
        let target_index = pattern::binding_index(&planned.plan, target_schema, *binding_id)
            .ok_or(ExecutorError::ImplementationDefined {
                detail: "subquery outer binding missing from target row",
            })?;
        values[target_index] = row.get(source_index).cloned().unwrap_or(Value::Null);
    }
    Ok(Binding::new(values))
}
