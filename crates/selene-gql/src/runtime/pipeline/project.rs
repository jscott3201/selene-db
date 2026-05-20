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
    let mut rows = Vec::with_capacity(input_rows.len());
    let mut rows_since_check = 0;
    for row in input_rows {
        ctx.tx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let values = items
            .iter()
            .map(|item| project_value(item, &row, &input_schema, ctx))
            .collect::<Result<Vec<_>, _>>()?;
        rows.push(Binding::new(values));
    }
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
