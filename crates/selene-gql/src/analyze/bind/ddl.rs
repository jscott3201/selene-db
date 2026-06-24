//! DDL bind handling.

use crate::{DdlStatement, TypePropertyConstraint};

use super::{BindContext, expr};
use crate::analyze::error::AnalysisError;

pub(crate) fn bind_ddl_statement(
    ctx: &mut BindContext,
    statement: &DdlStatement,
) -> Result<(), AnalysisError> {
    match statement {
        DdlStatement::CreateNodeType { properties, .. }
        | DdlStatement::CreateEdgeType { properties, .. }
        | DdlStatement::AlterEdgeType { properties, .. } => {
            for property in properties {
                for constraint in &property.constraints {
                    if let TypePropertyConstraint::Default(value, _) = constraint {
                        expr::bind_value_expr(ctx, value)?;
                    }
                }
            }
        }
        DdlStatement::CreateGraph { .. }
        | DdlStatement::DropGraph { .. }
        | DdlStatement::DropNodeType { .. }
        | DdlStatement::DropEdgeType { .. }
        | DdlStatement::TruncateNodeType { .. }
        | DdlStatement::TruncateEdgeType { .. }
        | DdlStatement::CreateIndex { .. }
        | DdlStatement::DropIndex { .. }
        | DdlStatement::ShowNodeTypes(_)
        | DdlStatement::ShowEdgeTypes(_)
        | DdlStatement::ShowIndexes(_)
        | DdlStatement::ShowProcedures(_) => {}
    }
    Ok(())
}
