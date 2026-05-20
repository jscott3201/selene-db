//! `selene.health` built-in.

use selene_gql::{
    GqlType, GraphContext, ProcedureError, ProcedureMutability, ProcedureResult, ProcedureTier,
    Value,
};

use crate::builtin::{BuiltInMetadata, GraphProcedureBuiltIn, StaticOutputColumn, StaticParameter};

static HEALTH_OUTPUTS: [StaticOutputColumn; 4] = [
    StaticOutputColumn::new("graph_id", GqlType::Uint64),
    StaticOutputColumn::new("node_count", GqlType::Uint64),
    StaticOutputColumn::new("edge_count", GqlType::Uint64),
    StaticOutputColumn::new("schema_bound", GqlType::Boolean),
];

/// Built-in read-only graph health procedure.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SeleneHealth;

impl BuiltInMetadata for SeleneHealth {
    fn name(&self) -> &'static [&'static str] {
        &["selene", "health"]
    }

    fn tier(&self) -> ProcedureTier {
        ProcedureTier::Graph
    }

    fn mutability(&self) -> ProcedureMutability {
        ProcedureMutability::Read
    }

    fn signature_static(&self) -> &'static [StaticParameter] {
        &[]
    }

    fn output_columns_static(&self) -> &'static [StaticOutputColumn] {
        &HEALTH_OUTPUTS
    }
}

impl GraphProcedureBuiltIn for SeleneHealth {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        if !args.is_empty() {
            return Err(ProcedureError::InvalidArgument {
                detail: "selene.health expects zero arguments".to_owned(),
            });
        }

        let snapshot = ctx.snapshot();
        Ok(ProcedureResult {
            rows: vec![vec![
                Value::Uint(snapshot.graph_id().get()),
                Value::Uint(snapshot.node_count() as u64),
                Value::Uint(snapshot.edge_count() as u64),
                Value::Bool(snapshot.meta.bound_type.is_some()),
            ]],
        })
    }
}
