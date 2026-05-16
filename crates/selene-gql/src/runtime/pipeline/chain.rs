//! NEXT-chain pipeline operator.
//!
//! NEXT establishes a fresh binding-table segment. The lhs binding table is
//! discarded; the rhs `ExecutionPlan` runs standalone. Correlated NEXT (rhs
//! references prior-block bindings) is rejected at plan time by
//! `plan::lowering::lower_chained` and surfaces as
//! `PlannerError::NotImplemented`.

use crate::{
    ExecutionPlan,
    runtime::{BindingTable, ExecutorError, TxContext, execute_plan},
};

pub(super) fn execute(
    rhs: &ExecutionPlan,
    _input: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    execute_plan(rhs, ctx)
}
