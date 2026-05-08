//! Procedure-call bind handling.

use selene_core::IStr;

use super::{BindContext, expr};
use crate::{
    ProcedureCall, ProcedureOutputColumn, YieldColumn,
    analyze::{
        binding::BindingDeclKind,
        error::{AnalysisError, ExpectedType, TypeMismatchContext},
        infer,
        types::AnalyzedType,
    },
};

pub(crate) fn bind_procedure_call(
    ctx: &mut BindContext,
    call: &ProcedureCall,
) -> Result<(), AnalysisError> {
    let Some(metadata) = ctx.registry().lookup(&call.name) else {
        return Err(AnalysisError::UnknownProcedure {
            name: boxed_name(call),
            span: call.span,
        });
    };

    let mut arg_types = Vec::with_capacity(call.args.len());
    for arg in &call.args {
        let id = expr::bind_value_expr(ctx, arg)?;
        arg_types.push((ctx.expr_type(id).clone(), arg.span()));
    }

    let expected = metadata.signature.parameters.len();
    let actual = arg_types.len();
    if actual != expected {
        return Err(AnalysisError::WrongArgumentCount {
            procedure: boxed_name(call),
            expected,
            actual,
            span: call.span,
        });
    }

    for ((arg_ty, span), (position, parameter)) in arg_types
        .iter()
        .zip(metadata.signature.parameters.iter().enumerate())
    {
        if let AnalyzedType::Resolved(found) = arg_ty
            && !infer::argument_assignable(found, &parameter.ty, parameter.nullable)
        {
            return Err(AnalysisError::TypeMismatch {
                context: TypeMismatchContext::ProcedureArgument {
                    procedure: boxed_name(call),
                    parameter: parameter.name,
                    position,
                },
                expected: ExpectedType::Specific(parameter.ty.clone()),
                found: found.clone(),
                span: *span,
            });
        }
    }

    // `YIELD *` is a Selene extension. Expand wildcard columns first in the
    // registered schema order, then process explicit named items in source
    // order. Duplicate output names naturally fail through strict declaration.
    for item in &call.yield_items {
        match item.column {
            YieldColumn::Star => {
                for column in &metadata.output_schema.columns {
                    declare_output(ctx, column, column.name, item.span)?;
                }
            }
            YieldColumn::Named(column) => {
                let Some(output) = metadata
                    .output_schema
                    .columns
                    .iter()
                    .find(|candidate| candidate.name == column)
                else {
                    return Err(AnalysisError::UnknownYieldColumn {
                        procedure: boxed_name(call),
                        column,
                        span: item.span,
                    });
                };
                let name = item.alias.unwrap_or(column);
                declare_output(ctx, output, name, item.span)?;
            }
        }
    }
    Ok(())
}

fn declare_output(
    ctx: &mut BindContext,
    column: &ProcedureOutputColumn,
    name: IStr,
    span: crate::SourceSpan,
) -> Result<(), AnalysisError> {
    ctx.declare_strict_typed(
        BindingDeclKind::YieldColumn,
        name,
        span,
        AnalyzedType::Resolved(column.ty.clone()),
    )?;
    Ok(())
}

fn boxed_name(call: &ProcedureCall) -> Box<[IStr]> {
    call.name.clone().into_boxed_slice()
}
