use selene_core::Value;

use crate::{
    FilterPredicate,
    runtime::{BindingTable, EvalCtx, ExecutorError, evaluator},
};

pub(super) fn execute(
    predicate: &FilterPredicate,
    table: BindingTable,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    if predicate.index_consumed {
        return Err(ExecutorError::ImplementationDefined {
            detail: "index-consumed predicate emitted into pipeline",
        });
    }

    let (schema, input_rows) = table.into_parts();
    let mut rows = Vec::new();
    let mut rows_since_check = 0;
    for row in input_rows {
        ctx.tx.check_cancellation_stride(&mut rows_since_check, 1)?;
        match evaluator::evaluate(&predicate.expr, &row, &schema, ctx)? {
            Value::Bool(true) => rows.push(row),
            Value::Bool(false) | Value::Null => {}
            _ => {}
        }
    }
    Ok(BindingTable::new(schema, rows))
}
