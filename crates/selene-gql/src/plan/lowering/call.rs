//! Procedure-call lowering.

use crate::{
    ProcedureCall, ProcedureOutputColumn, ProcedureRegistry, YieldColumn,
    analyze::{AnalyzedStatement, AnalyzedType},
    plan::{
        BindingTableColumn, BindingTableSchema, ExecutionPlan, ImplDefinedCaps, PipelineOp,
        PlannedCall, PlannedYieldItem, PlannerError, YieldKind,
    },
};

use super::expr;

/// Lower a top-level CALL statement.
pub(crate) fn lower_top_level_call(
    call: &ProcedureCall,
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
) -> Result<ExecutionPlan, PlannerError> {
    let planned = plan_call(call, registry, analyzed)?;
    let columns = yield_to_columns(&planned)?;
    Ok(ExecutionPlan {
        pattern_plan: None,
        pipeline: vec![PipelineOp::Call(planned)],
        output_schema: BindingTableSchema { columns },
        impl_defined_caps: ImplDefinedCaps::default(),
    })
}

/// Lower a procedure call into a planned call payload.
pub(crate) fn plan_call(
    call: &ProcedureCall,
    registry: &dyn ProcedureRegistry,
    analyzed: &AnalyzedStatement,
) -> Result<PlannedCall, PlannerError> {
    let metadata = registry
        .lookup(&call.name)
        .ok_or_else(|| PlannerError::UnknownProcedure {
            procedure: call.name.clone().into_boxed_slice(),
            span: call.span,
        })?;

    if metadata.signature.parameters.len() != call.args.len() {
        return Err(PlannerError::ProcedureMetadataMismatch {
            procedure: call.name.clone().into_boxed_slice(),
            detail: "signature parameter count changed",
            span: call.span,
        });
    }

    let args = call
        .args
        .iter()
        .map(|arg| expr::project_expr(arg, None, analyzed))
        .collect::<Result<Vec<_>, _>>()?;
    let yield_cols = call
        .yield_items
        .iter()
        .map(|item| PlannedYieldItem {
            column: match item.column {
                YieldColumn::Star => YieldKind::Star,
                YieldColumn::Named(name) => YieldKind::Named(name),
            },
            alias: item.alias,
            span: item.span,
        })
        .collect();

    Ok(PlannedCall {
        procedure: call.name.clone().into_boxed_slice(),
        handle: metadata.handle,
        args,
        yield_cols,
        output_schema: metadata.output_schema,
        mutability: metadata.mutability,
        span: call.span,
    })
}

/// Convert planned yield items into binding-table columns.
pub(crate) fn yield_to_columns(
    planned: &PlannedCall,
) -> Result<Vec<BindingTableColumn>, PlannerError> {
    if planned.yield_cols.is_empty() {
        return Ok(Vec::new());
    }

    let mut columns = Vec::new();
    if planned
        .yield_cols
        .iter()
        .any(|item| matches!(item.column, YieldKind::Star))
    {
        for col in &planned.output_schema.columns {
            columns.push(binding_column(col, col.name));
        }
    }

    for item in &planned.yield_cols {
        let YieldKind::Named(name) = item.column else {
            continue;
        };
        let col = planned
            .output_schema
            .columns
            .iter()
            .find(|candidate| candidate.name == name)
            .ok_or_else(|| PlannerError::ProcedureMetadataMismatch {
                procedure: planned.procedure.clone(),
                detail: "yield column not in registry output schema",
                span: item.span,
            })?;
        columns.push(binding_column(col, item.alias.unwrap_or(name)));
    }

    Ok(columns)
}

fn binding_column(col: &ProcedureOutputColumn, name: selene_core::IStr) -> BindingTableColumn {
    BindingTableColumn {
        name: Some(name),
        ty: AnalyzedType::Resolved(col.ty.clone()),
    }
}

#[cfg(test)]
mod defensive_tests {
    use selene_core::intern_with_admission;

    use super::*;
    use crate::{
        GqlType, ProcedureHandle, ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn,
        ProcedureOutputSchema, ProcedureRegistry, ProcedureSignature, ProcedureTier, SourceSpan,
        procedure_registry::{ProcedureError, ProcedureResult, Value},
    };

    #[derive(Clone, Copy)]
    struct ChangedOutputRegistry;

    impl ProcedureRegistry for ChangedOutputRegistry {
        fn lookup(&self, _name: &[selene_core::IStr]) -> Option<ProcedureMetadata> {
            Some(ProcedureMetadata {
                handle: ProcedureHandle::new(1),
                signature: ProcedureSignature::default(),
                output_schema: ProcedureOutputSchema {
                    columns: vec![ProcedureOutputColumn {
                        name: intern_with_admission("different").expect("test interner").0,
                        ty: GqlType::String,
                    }],
                },
                tier: ProcedureTier::Graph,
                mutability: ProcedureMutability::Read,
                capability_required: None,
            })
        }

        fn execute(
            &self,
            _handle: ProcedureHandle,
            _args: &[Value],
        ) -> Result<ProcedureResult, ProcedureError> {
            Err(ProcedureError::M2Placeholder)
        }
    }

    #[test]
    fn changed_yield_schema_reports_metadata_mismatch() {
        let name = intern_with_admission("pkg").expect("test interner").0;
        let col = intern_with_admission("out").expect("test interner").0;
        let planned = PlannedCall {
            procedure: Box::new([name]),
            handle: ProcedureHandle::new(1),
            args: Vec::new(),
            yield_cols: vec![PlannedYieldItem {
                column: YieldKind::Named(col),
                alias: None,
                span: SourceSpan::new(0, 3),
            }],
            output_schema: ChangedOutputRegistry
                .lookup(&[name])
                .expect("metadata")
                .output_schema,
            mutability: ProcedureMutability::Read,
            span: SourceSpan::new(0, 3),
        };

        let err = yield_to_columns(&planned).expect_err("metadata changed");
        assert!(matches!(
            err,
            PlannerError::ProcedureMetadataMismatch {
                detail: "yield column not in registry output schema",
                ..
            }
        ));
    }
}
