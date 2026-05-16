use crate::{
    BindingTableColumn, BindingTableSchema, ProjectExpr,
    runtime::{Binding, BindingTable, ExecutorError, TxContext, evaluator},
};

pub(super) fn execute(
    items: &[ProjectExpr],
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (input_schema, input_rows) = table.into_parts();
    let new_columns = items
        .iter()
        .map(|item| BindingTableColumn {
            name: item.alias,
            hidden: None,
            ty: item.ty.clone(),
        })
        .collect::<Vec<_>>();
    let prefix_schemas = prefix_schemas(&input_schema, &new_columns);
    let mut output_schema = input_schema.clone();
    output_schema.columns.extend(new_columns.iter().cloned());

    let rows = input_rows
        .into_iter()
        .map(|row| {
            let mut values = row.values().to_vec();
            for (index, item) in items.iter().enumerate() {
                let current_row = Binding::new(values.clone());
                let value =
                    evaluator::evaluate(&item.expr, &current_row, &prefix_schemas[index], ctx)?;
                values.push(value);
            }
            Ok(Binding::new(values))
        })
        .collect::<Result<Vec<_>, ExecutorError>>()?;
    Ok(BindingTable::new(output_schema, rows))
}

fn prefix_schemas(
    input_schema: &BindingTableSchema,
    new_columns: &[BindingTableColumn],
) -> Vec<BindingTableSchema> {
    let mut schemas = Vec::with_capacity(new_columns.len());
    let mut current = input_schema.clone();
    for column in new_columns {
        schemas.push(current.clone());
        current.columns.push(column.clone());
    }
    schemas
}
