use selene_core::Value;

use crate::{
    FilterPredicate,
    runtime::{BindingTable, ExecutorError, TxContext, evaluator},
};

pub(super) fn execute(
    predicate: &FilterPredicate,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    if predicate.index_consumed {
        return Err(ExecutorError::ImplementationDefined {
            detail: "index-consumed predicate emitted into pipeline",
        });
    }

    let (schema, input_rows) = table.into_parts();
    let rows = input_rows
        .into_iter()
        .filter_map(
            |row| match evaluator::evaluate(&predicate.expr, &row, &schema, ctx) {
                Ok(Value::Bool(true)) => Some(Ok(row)),
                Ok(Value::Bool(false) | Value::Null) => None,
                Ok(_) => None,
                Err(err) => Some(Err(err)),
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BindingTable::new(schema, rows))
}
