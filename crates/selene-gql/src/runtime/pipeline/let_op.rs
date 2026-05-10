use crate::{
    BindingTableColumn, ProjectExpr,
    runtime::{Binding, BindingTable, ExecutorError, TxContext, evaluator},
};

pub(super) fn execute(
    items: &[ProjectExpr],
    table: BindingTable,
    ctx: &TxContext<'_>,
) -> Result<BindingTable, ExecutorError> {
    let input_schema = table.schema().clone();
    let mut output_schema = input_schema.clone();
    output_schema
        .columns
        .extend(items.iter().map(|item| BindingTableColumn {
            name: item.alias,
            hidden: None,
            ty: item.ty.clone(),
        }));

    let rows = table
        .rows()
        .iter()
        .map(|row| {
            let mut values = row.values().to_vec();
            for item in items {
                values.push(evaluator::evaluate(&item.expr, row, &input_schema, ctx)?);
            }
            Ok(Binding::new(values))
        })
        .collect::<Result<Vec<_>, ExecutorError>>()?;
    Ok(BindingTable::new(output_schema, rows))
}
