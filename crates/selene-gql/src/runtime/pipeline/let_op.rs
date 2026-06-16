use crate::{
    BindingTableColumn, BindingTableSchema, ProjectExpr,
    runtime::{Binding, BindingTable, EvalCtx, ExecutorError, evaluator, parameter_type},
};

pub(super) fn execute(
    items: &[ProjectExpr],
    table: BindingTable,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (input_schema, input_rows) = table.into_parts();
    let new_columns = items
        .iter()
        .map(|item| BindingTableColumn {
            name: item.alias.clone(),
            hidden: None,
            ty: item.ty.clone(),
        })
        .collect::<Vec<_>>();
    let prefix_schemas = prefix_schemas(&input_schema, &new_columns);
    let mut output_schema = input_schema.clone();
    output_schema.columns.extend(new_columns.iter().cloned());

    let mut rows = Vec::with_capacity(input_rows.len());
    let mut rows_since_check = 0;
    for row in input_rows {
        ctx.tx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let mut values = row.values().to_vec();
        for (index, item) in items.iter().enumerate() {
            let current_row = Binding::new(values.clone());
            let value = evaluator::evaluate(&item.expr, &current_row, &prefix_schemas[index], ctx)?;
            if let (Some(declared_type), Some(alias)) = (&item.declared_type, &item.alias) {
                parameter_type::validate_declared_type(
                    alias.clone(),
                    &value,
                    declared_type,
                    item.span,
                )?;
            }
            values.push(value);
        }
        rows.push(Binding::new(values));
    }
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
