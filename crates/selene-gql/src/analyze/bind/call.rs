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

    // Mixed `YIELD *, named AS alias` is legal: the parser already accepts
    // it (see crates/selene-gql/tests/write_side.rs). Bind every explicit
    // named column regardless of whether a wildcard accompanies them; the
    // wildcard is recorded separately so BRIEF-23 can expand the
    // registry-driven additional columns at the same scope position.
    for item in &call.yield_items {
        match item.column {
            YieldColumn::Star => ctx.record_yield_star(item.span),
            YieldColumn::Named(column) => {
                let name = item.alias.unwrap_or(column);
                ctx.declare_strict(BindingDeclKind::YieldColumn, name, item.span)?;
            }
        }
    }
    Ok(())
}
