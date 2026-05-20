use crate::{
    BindingTableColumn, BindingTableSchema, ProjectExpr,
    runtime::{Binding, BindingTable, EvalCtx, ExecutorError, evaluator},
};

pub(super) fn execute(
    items: &[ProjectExpr],
    table: BindingTable,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (input_schema, input_rows) = table.into_parts();
    let output_schema = schema_for_items(items);
    let rows = input_rows
        .into_iter()
        .map(|row| {
            let values = items
                .iter()
                .map(|item| project_value(item, &row, &input_schema, ctx))
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
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<selene_core::Value, ExecutorError> {
    evaluator::evaluate(&item.expr, row, schema, ctx)
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
