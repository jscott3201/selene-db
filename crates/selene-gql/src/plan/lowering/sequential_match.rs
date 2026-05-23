//! Non-leading MATCH lowering helpers.

use crate::{
    MatchClause,
    analyze::AnalyzedStatement,
    plan::{BindingTableColumn, PipelineOp, PlannerError},
};

use super::{match_clause, visible_after_pattern};

pub(super) fn lower(
    clause: &MatchClause,
    analyzed: &AnalyzedStatement,
    ops: &mut Vec<PipelineOp>,
    visible: &mut Vec<BindingTableColumn>,
) -> Result<(), PlannerError> {
    let pattern = match_clause::lower_match_prefix(&[clause], analyzed)?.ok_or(
        PlannerError::NotImplemented {
            feature: "empty graph pattern",
            span: clause.span,
        },
    )?;
    for column in visible_after_pattern(Some(&pattern)) {
        if !visible.iter().any(|existing| existing.name == column.name) {
            visible.push(column);
        }
    }
    ops.push(PipelineOp::Match(pattern));
    Ok(())
}
