//! Procedure-call bind handling.

use selene_core::IStr;

use super::{BindContext, expr};
use crate::{
    ProcedureCall, ProcedureMetadata, ProcedureOutputColumn, YieldColumn,
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
    let metadata = lookup_metadata(ctx, call)?;
    bind_procedure_call_with_metadata(ctx, call, metadata)
}

pub(crate) fn lookup_metadata(
    ctx: &BindContext,
    call: &ProcedureCall,
) -> Result<ProcedureMetadata, AnalysisError> {
    let procedure = call.name.clone().into_boxed_slice();
    ctx.registry()
        .lookup(&call.name)
        .ok_or(AnalysisError::UnknownProcedure {
            name: procedure,
            span: call.span,
        })
}

pub(crate) fn bind_procedure_call_with_metadata(
    ctx: &mut BindContext,
    call: &ProcedureCall,
    metadata: ProcedureMetadata,
) -> Result<(), AnalysisError> {
    let procedure = call.name.clone().into_boxed_slice();

    let expected = metadata.signature.parameters.len();
    let actual = call.args.len();
    if actual != expected {
        return Err(AnalysisError::WrongArgumentCount {
            procedure,
            expected,
            actual,
            span: call.span,
        });
    }

    let mut arg_types = Vec::with_capacity(call.args.len());
    for arg in &call.args {
        let id = expr::bind_value_expr(ctx, arg)?;
        arg_types.push((ctx.expr_type(id).clone(), arg.span()));
    }

    for ((arg_ty, span), (position, parameter)) in arg_types
        .iter()
        .zip(metadata.signature.parameters.iter().enumerate())
    {
        // Dynamic operand defers per BRIEF-22.
        if let AnalyzedType::Resolved(found) = arg_ty
            && !infer::argument_assignable(found, &parameter.ty, parameter.nullable)
        {
            return Err(AnalysisError::TypeMismatch {
                context: TypeMismatchContext::ProcedureArgument {
                    procedure,
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
    if let Some(star_span) = call
        .yield_items
        .iter()
        .find(|item| matches!(item.column, YieldColumn::Star))
        .map(|item| item.span)
    {
        for column in &metadata.output_schema.columns {
            declare_output(ctx, column, column.name, star_span)?;
        }
    }

    for item in &call.yield_items {
        if let YieldColumn::Named(column) = item.column {
            let Some(output) = metadata
                .output_schema
                .columns
                .iter()
                .find(|candidate| candidate.name == column)
            else {
                return Err(AnalysisError::UnknownYieldColumn {
                    procedure,
                    column,
                    span: item.span,
                });
            };
            let name = item.alias.unwrap_or(column);
            declare_output(ctx, output, name, item.span)?;
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
