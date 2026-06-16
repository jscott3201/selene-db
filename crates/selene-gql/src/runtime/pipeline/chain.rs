//! NEXT-chain pipeline operator.
//!
//! NEXT establishes a fresh binding-table segment. The lhs binding table is
//! discarded for uncorrelated blocks. Correlated NEXT runs the rhs
//! `ExecutionPlan` once per input row with that row available as the seed.

use crate::{
    ExecutionPlan,
    runtime::{BindingTable, ExecutorError, TxContext, execute_plan, plan_runner},
};

pub(super) fn execute(
    rhs: &ExecutionPlan,
    _input: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    ctx.check_cancellation()?;
    execute_plan(rhs, ctx)
}

pub(super) fn execute_correlated(
    rhs: &ExecutionPlan,
    input: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (schema, rows) = input.into_parts();
    let mut output = Vec::new();
    let mut rows_since_check = 0;
    for row in rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let table = plan_runner::execute_plan_with_seed(
            rhs,
            Some(BindingTable::new(schema.clone(), vec![row])),
            ctx,
        )?;
        output.extend(table.into_parts().1);
    }
    Ok(BindingTable::new(rhs.output_schema.clone(), output))
}

pub(super) fn execute_correlated_read_only(
    rhs: &ExecutionPlan,
    input: BindingTable,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (schema, rows) = input.into_parts();
    let mut output = Vec::new();
    let mut rows_since_check = 0;
    for row in rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let table = plan_runner::execute_plan_read_only_with_seed(
            rhs,
            Some(BindingTable::new(schema.clone(), vec![row])),
            ctx,
        )?;
        output.extend(table.into_parts().1);
    }
    Ok(BindingTable::new(rhs.output_schema.clone(), output))
}
