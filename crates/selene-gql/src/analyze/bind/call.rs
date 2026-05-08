//! Procedure-call bind handling.

use crate::{ProcedureCall, YieldColumn};

use super::{BindContext, expr};
use crate::analyze::{binding::BindingDeclKind, error::AnalysisError};

pub(crate) fn bind_procedure_call(
    ctx: &mut BindContext,
    call: &ProcedureCall,
) -> Result<(), AnalysisError> {
    for arg in &call.args {
        expr::bind_value_expr(ctx, arg)?;
    }

    let has_star = call
        .yield_items
        .iter()
        .any(|item| matches!(item.column, YieldColumn::Star));
    if has_star {
        ctx.record_yield_star(call.span);
        return Ok(());
    }

    for item in &call.yield_items {
        let YieldColumn::Named(column) = item.column else {
            continue;
        };
        let name = item.alias.unwrap_or(column);
        ctx.declare_strict(BindingDeclKind::YieldColumn, name, item.span)?;
    }
    Ok(())
}
