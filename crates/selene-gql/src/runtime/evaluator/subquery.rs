//! Planned expression-subquery evaluation.
//!
//! `EXISTS { MATCH ... }` follows ISO/IEC 39075:2024 section 19.4 and is
//! two-valued: it returns `TRUE` when the inner pattern has at least one row and
//! `FALSE` otherwise. `COUNT { MATCH ... }` is a selene-db dialect extension
//! over the same planned single-MATCH surface. Correlated outer bindings are
//! projected into the inner pattern's seed row before execution.

use selene_core::Value;

use crate::{
    BindingTableColumn, SourceSpan, ValueExpr,
    plan::{OuterBindingRef, PlannedSubquery},
    runtime::{
        Binding, BindingTable, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError,
        pattern,
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

fn execute_subquery(
    expr: &ValueExpr,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let planned = planned_subquery(expr, ctx)?;
    let target_schema = target_schema(planned, schema)?;
    let Some(seed) = seed_binding(planned, binding, schema, &target_schema)? else {
        return Ok(BindingTable::new(target_schema, Vec::new()));
    };
    pattern::execute_pattern_with_seed_and_schema(&planned.plan, Some(&seed), target_schema, ctx)
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
) -> Result<Option<Binding>, ExecutorError> {
    let mut values = vec![Value::Null; target_schema.columns.len()];
    for outer in &planned.outer_binding_refs {
        let source_index = source_index(source_schema, outer)?;
        let value = row.get(source_index).cloned().unwrap_or(Value::Null);
        if matches!(value, Value::Null) {
            return Ok(None);
        }
        let target_index = pattern::column_index(target_schema, outer.name).ok_or(
            ExecutorError::ImplementationDefined {
                detail: "subquery outer binding missing from target row",
            },
        )?;
        values[target_index] = value;
    }
    Ok(Some(Binding::new(values)))
}

fn target_schema(
    planned: &PlannedSubquery,
    source_schema: &BindingTableSchema,
) -> Result<BindingTableSchema, ExecutorError> {
    let mut schema = pattern::schema_for_pattern(&planned.plan);
    for outer in &planned.outer_binding_refs {
        if pattern::column_index(&schema, outer.name).is_some() {
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
    Ok(schema)
}

fn source_index(
    source_schema: &BindingTableSchema,
    outer: &OuterBindingRef,
) -> Result<usize, ExecutorError> {
    pattern::column_index(source_schema, outer.name).ok_or(ExecutorError::ImplementationDefined {
        detail: "subquery outer binding missing from source row",
    })
}
