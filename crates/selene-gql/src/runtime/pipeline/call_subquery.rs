//! Inline `CALL { ... }` table-subquery pipeline operator.

use selene_core::Value;

use crate::{
    BindingTableColumn, BindingTableSchema, PlannedTableSubquery,
    runtime::{Binding, BindingTable, ExecutorError, TxContext, pattern, plan_runner},
};

pub(super) fn execute(
    call: &PlannedTableSubquery,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    execute_read_only(call, table, ctx)
}

pub(super) fn execute_read_only(
    call: &PlannedTableSubquery,
    table: BindingTable,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (input_schema, input_rows) = table.into_parts();
    let output_schema = output_schema(&input_schema, call);
    let mut output = Vec::new();
    let mut rows_since_check = 0;

    for row in input_rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let target_schema = target_schema(call, &input_schema)?;
        let Some(seed) = seed_binding(call, &row, &input_schema, &target_schema)? else {
            continue;
        };
        let inner = plan_runner::execute_plan_read_only_with_seed(
            &call.body,
            Some(BindingTable::new(target_schema, vec![seed])),
            ctx,
        )?;
        let yield_indices = yield_indices(call, inner.schema())?;
        for inner_row in inner.rows() {
            ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
            let mut values = row.values().to_vec();
            for index in &yield_indices {
                values.push(inner_row.get(*index).cloned().unwrap_or(Value::Null));
            }
            output.push(Binding::with_insert_sites(
                values,
                row.insert_sites().iter().copied().collect(),
            ));
        }
    }

    Ok(BindingTable::new(output_schema, output))
}

fn output_schema(
    input_schema: &BindingTableSchema,
    call: &PlannedTableSubquery,
) -> BindingTableSchema {
    let mut schema = input_schema.clone();
    schema.columns.extend(call.yield_schema.clone());
    schema
}

fn target_schema(
    call: &PlannedTableSubquery,
    source_schema: &BindingTableSchema,
) -> Result<BindingTableSchema, ExecutorError> {
    let mut schema = call
        .body
        .pattern_plan
        .as_ref()
        .map(pattern::schema_for_pattern)
        .unwrap_or_else(|| BindingTableSchema {
            columns: Vec::new(),
        });
    for outer in &call.outer_binding_refs {
        if pattern::column_index(&schema, outer.name).is_some() {
            continue;
        }
        let source_index = source_index(source_schema, outer.name)?;
        let source_column = &source_schema.columns[source_index];
        schema.columns.push(BindingTableColumn {
            name: Some(outer.name),
            hidden: None,
            ty: source_column.ty.clone(),
        });
    }
    Ok(schema)
}

fn seed_binding(
    call: &PlannedTableSubquery,
    row: &Binding,
    source_schema: &BindingTableSchema,
    target_schema: &BindingTableSchema,
) -> Result<Option<Binding>, ExecutorError> {
    let mut values = vec![Value::Null; target_schema.columns.len()];
    for outer in &call.outer_binding_refs {
        let source_index = source_index(source_schema, outer.name)?;
        let value = row.get(source_index).cloned().unwrap_or(Value::Null);
        if matches!(value, Value::Null) {
            return Ok(None);
        }
        let target_index = pattern::column_index(target_schema, outer.name).ok_or(
            ExecutorError::ImplementationDefined {
                detail: "CALL subquery outer binding missing from target row",
            },
        )?;
        values[target_index] = value;
    }
    Ok(Some(Binding::new(values)))
}

fn yield_indices(
    call: &PlannedTableSubquery,
    inner_schema: &BindingTableSchema,
) -> Result<Vec<usize>, ExecutorError> {
    call.yield_items
        .iter()
        .map(|item| source_index(inner_schema, item.source))
        .collect()
}

fn source_index(
    schema: &BindingTableSchema,
    name: selene_core::IStr,
) -> Result<usize, ExecutorError> {
    pattern::column_index(schema, name).ok_or(ExecutorError::ImplementationDefined {
        detail: "CALL subquery binding missing from source row",
    })
}
