//! Subplan join-tree operator.

use crate::{
    ExecutionPlan,
    runtime::{Binding, BindingTableSchema, ExecutorError, TxContext},
};

use super::pattern;

pub(crate) fn execute(
    plan: &ExecutionPlan,
    schema: &BindingTableSchema,
    seed: Option<&Binding>,
    ctx: &TxContext<'_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    ensure_phase_a_compatible(plan, seed)?;
    let Some(pattern_plan) = &plan.pattern_plan else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "Subplan without pattern plan",
        });
    };
    let table = pattern::execute_pattern(pattern_plan, ctx)?;
    Ok(table
        .rows()
        .iter()
        .map(|row| pattern::project_row_to_schema(row, table.schema(), schema, seed))
        .collect())
}

fn ensure_phase_a_compatible(
    plan: &ExecutionPlan,
    seed: Option<&Binding>,
) -> Result<(), ExecutorError> {
    if seed.is_some() {
        return Err(ExecutorError::ImplementationDefined {
            detail: "correlated subplans not yet supported",
        });
    }
    if !plan.pipeline.is_empty() {
        return Err(ExecutorError::ImplementationDefined {
            detail: "Subplan pipeline ops not yet supported",
        });
    }
    Ok(())
}
