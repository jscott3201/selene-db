//! Set-composition pipeline operator.

use crate::{
    ExecutionPlan, SetOp, SourceSpan,
    runtime::{BindingTable, DataExceptionSubclass, ExecutorError, TxContext, execute_plan},
};

use super::distinct;

pub(super) fn execute(
    op: SetOp,
    rhs: &ExecutionPlan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    match op {
        SetOp::Union | SetOp::UnionAll => {
            let rhs_table = execute_plan(rhs, ctx)?;
            assert_compatible_schemas(&table, &rhs_table)?;
            let (schema, mut rows) = table.into_parts();
            ctx.check_cancellation()?;
            rows.extend(rhs_table.rows().iter().cloned());
            let combined = BindingTable::new(schema, rows);
            if matches!(op, SetOp::Union) {
                distinct::execute(combined, ctx)
            } else {
                Ok(combined)
            }
        }
        SetOp::Intersect
        | SetOp::IntersectAll
        | SetOp::Except
        | SetOp::ExceptAll
        | SetOp::Otherwise => Err(ExecutorError::ImplementationDefined {
            detail: "set operator not implemented",
        }),
    }
}

fn assert_compatible_schemas(lhs: &BindingTable, rhs: &BindingTable) -> Result<(), ExecutorError> {
    let lhs_len = lhs.schema().columns.len();
    let rhs_len = rhs.schema().columns.len();
    if lhs_len != rhs_len {
        return Err(ExecutorError::DataException {
            subclass: DataExceptionSubclass::DataException,
            message: format!(
                "UNION arms have differing column counts: lhs={lhs_len}, rhs={rhs_len}"
            ),
            span: SourceSpan::default(),
        });
    }
    Ok(())
}
