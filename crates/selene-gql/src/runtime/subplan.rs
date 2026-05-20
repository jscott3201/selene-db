//! Subplan join-tree operator.

use crate::{
    ExecutionPlan,
    runtime::{Binding, BindingTableSchema, EvalCtx, ExecutorError},
};

use super::pattern;

pub(crate) fn execute(
    plan: &ExecutionPlan,
    schema: &BindingTableSchema,
    seed: Option<&Binding>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    ensure_phase_a_compatible(plan)?;
    let Some(pattern_plan) = &plan.pattern_plan else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "Subplan without pattern plan",
        });
    };
    let sub_ctx = ctx.with_plan(&plan.expr_ids, &plan.subqueries);
    let table = pattern::execute_pattern_with_seed(pattern_plan, seed, &sub_ctx)?;
    Ok(table
        .rows()
        .iter()
        .map(|row| pattern::project_row_to_schema(row, table.schema(), schema, seed))
        .collect())
}

fn ensure_phase_a_compatible(plan: &ExecutionPlan) -> Result<(), ExecutorError> {
    if !plan.pipeline.is_empty() {
        return Err(ExecutorError::ImplementationDefined {
            detail: "Subplan pipeline ops not yet supported",
        });
    }
    Ok(())
}
