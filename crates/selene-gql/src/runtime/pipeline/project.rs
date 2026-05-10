use crate::{
    BindingTableColumn, BindingTableSchema, ProjectExpr, ValueExpr,
    runtime::{Binding, BindingTable, ExecutorError, TxContext, evaluator},
};

pub(super) fn execute(
    items: &[ProjectExpr],
    table: BindingTable,
    ctx: &TxContext<'_>,
) -> Result<BindingTable, ExecutorError> {
    let input_schema = table.schema().clone();
    let output_schema = schema_for_items(items);
    let rows = table
        .rows()
        .iter()
        .map(|row| {
            let values = items
                .iter()
                .map(|item| project_value(item, row, &input_schema, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Binding::new(values))
        })
        .collect::<Result<Vec<_>, ExecutorError>>()?;
    Ok(BindingTable::new(output_schema, rows))
}

fn project_value(
    item: &ProjectExpr,
    row: &Binding,
    schema: &BindingTableSchema,
    ctx: &TxContext<'_>,
) -> Result<selene_core::Value, ExecutorError> {
    match evaluator::evaluate(&item.expr, row, schema, ctx) {
        Ok(value) => Ok(value),
        Err(ExecutorError::ImplementationDefined { detail })
            if detail == "function call evaluation not implemented" =>
        {
            aggregate_column_value(item, row, schema)
                .ok_or(ExecutorError::ImplementationDefined { detail })
        }
        Err(err) => Err(err),
    }
}

fn aggregate_column_value(
    item: &ProjectExpr,
    row: &Binding,
    schema: &BindingTableSchema,
) -> Option<selene_core::Value> {
    let ValueExpr::FunctionCall { name, .. } = &item.expr else {
        return None;
    };
    if name.len() != 1 || !is_aggregate_name(name[0]) {
        return None;
    }
    item.alias
        .and_then(|alias| value_for_column(alias, row, schema))
        .or_else(|| value_for_column(name[0], row, schema))
}

fn value_for_column(
    name: selene_core::IStr,
    row: &Binding,
    schema: &BindingTableSchema,
) -> Option<selene_core::Value> {
    schema
        .columns
        .iter()
        .position(|column| column.name == Some(name))
        .and_then(|index| row.get(index).cloned())
}

fn is_aggregate_name(name: selene_core::IStr) -> bool {
    matches!(
        name.as_str(),
        "count" | "sum" | "avg" | "average" | "min" | "max" | "collect" | "collect_list"
    )
}

pub(super) fn schema_for_items(items: &[ProjectExpr]) -> BindingTableSchema {
    BindingTableSchema {
        columns: items
            .iter()
            .map(|item| BindingTableColumn {
                name: item.alias,
                hidden: None,
                ty: item.ty.clone(),
            })
            .collect(),
    }
}
