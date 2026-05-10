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
    let new_columns = items
        .iter()
        .map(|item| BindingTableColumn {
            name: item.alias,
            hidden: None,
            ty: item.ty.clone(),
        })
        .collect::<Vec<_>>();
    let mut output_schema = input_schema.clone();
    output_schema.columns.extend(new_columns.iter().cloned());

    let rows = table
        .rows()
        .iter()
        .map(|row| {
            let mut values = row.values().to_vec();
            let mut current_schema = input_schema.clone();
            for (index, item) in items.iter().enumerate() {
                let current_row = Binding::new(values.clone());
                let value = evaluator::evaluate(&item.expr, &current_row, &current_schema, ctx)?;
                values.push(value);
                current_schema.columns.push(new_columns[index].clone());
            }
            Ok(Binding::new(values))
        })
        .collect::<Result<Vec<_>, ExecutorError>>()?;
    Ok(BindingTable::new(output_schema, rows))
}
