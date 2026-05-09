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
    ctx: &TxContext<'_>,
) -> Result<Vec<Binding>, ExecutorError> {
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
