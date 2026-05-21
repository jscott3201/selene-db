use std::time::Instant;

use selene_core::metrics;

use crate::{
    PlannedCall, ProcedureError, ProcedureMutability, ProcedureTier,
    runtime::{ExecutorError, GraphContext, MutationContext, ProcedureContext, TxContext},
};

pub(super) fn validate_call_tier(call: &PlannedCall) -> Result<(), ExecutorError> {
    if call.tier == ProcedureTier::Persist {
        return Err(ExecutorError::ImplementationDefined {
            detail: "persist-tier procedures not implemented in v1.0",
        });
    }
    let expected = tier_for_mutability(call.mutability);
    if call.tier != expected {
        return Err(procedure_error(
            ProcedureError::TierMismatch {
                expected,
                actual: call.tier,
            },
            call.span,
            None,
        ));
    }
    Ok(())
}

pub(super) fn build<'borrow, 'ctx, 'g>(
    call: &PlannedCall,
    ctx: &'borrow mut TxContext<'ctx, 'g>,
) -> Result<ProcedureContext<'borrow, 'g>, ExecutorError>
where
    'ctx: 'borrow,
{
    match call.tier {
        ProcedureTier::Graph => Ok(ProcedureContext::Graph(GraphContext::new(
            ctx.snapshot(),
            ctx.impl_defined_caps(),
            ctx.providers(),
            ctx.cancellation_checker(),
        ))),
        ProcedureTier::Mutation => {
            let caps = ctx.impl_defined_caps();
            let cancellation = ctx.cancellation_checker();
            let mutator = ctx.mutator_with_span(
                "GraphWrite procedure requires a write transaction",
                call.span,
            )?;
            Ok(ProcedureContext::Mutation(MutationContext::new(
                mutator,
                caps,
                cancellation,
            )))
        }
        ProcedureTier::Persist => Err(ExecutorError::ImplementationDefined {
            detail: "persist-tier procedures not implemented in v1.0",
        }),
    }
}

pub(super) const fn tier_for_mutability(mutability: ProcedureMutability) -> ProcedureTier {
    match mutability {
        ProcedureMutability::Read => ProcedureTier::Graph,
        ProcedureMutability::GraphWrite
        | ProcedureMutability::SchemaWrite
        | ProcedureMutability::Admin => ProcedureTier::Mutation,
    }
}

pub(super) fn procedure_error(
    source: ProcedureError,
    span: crate::SourceSpan,
    deadline: Option<Instant>,
) -> ExecutorError {
    match source {
        ProcedureError::Cancelled => {
            metrics::counter_inc(metrics::CANCELLATIONS_TOTAL);
            ExecutorError::Cancelled { span }
        }
        ProcedureError::Timeout { elapsed } => {
            metrics::counter_inc(metrics::CANCELLATIONS_TOTAL);
            ExecutorError::Timeout {
                deadline: deadline.unwrap_or_else(Instant::now),
                elapsed,
                span,
            }
        }
        source => ExecutorError::Procedure { source, span },
    }
}
