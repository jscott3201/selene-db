use crate::{
    BindingTableColumn, BindingTableSchema, ProjectExpr,
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
                .map(|item| evaluator::evaluate(&item.expr, row, &input_schema, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Binding::new(values))
        })
        .collect::<Result<Vec<_>, ExecutorError>>()?;
    Ok(BindingTable::new(output_schema, rows))
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
