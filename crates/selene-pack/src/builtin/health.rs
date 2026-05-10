//! `selene.health` built-in.

use selene_gql::{
    GqlType, GraphContext, ProcedureError, ProcedureMutability, ProcedureResult, ProcedureTier,
    Value,
};

use crate::builtin::{BuiltInMetadata, GraphProcedureBuiltIn, StaticOutputColumn, StaticParameter};

static HEALTH_OUTPUTS: [StaticOutputColumn; 4] = [
    StaticOutputColumn {
        name: "graph_id",
        ty: GqlType::Integer,
    },
    StaticOutputColumn {
        name: "node_count",
        ty: GqlType::Integer,
    },
    StaticOutputColumn {
        name: "edge_count",
        ty: GqlType::Integer,
    },
    StaticOutputColumn {
        name: "schema_bound",
        ty: GqlType::Boolean,
    },
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
                i64_value(snapshot.graph_id().get(), "graph_id")?,
                i64_value(snapshot.node_count(), "node_count")?,
                i64_value(snapshot.edge_count(), "edge_count")?,
                Value::Bool(snapshot.meta.bound_type.is_some()),
            ]],
        })
    }
}

fn i64_value(value: impl TryInto<i64>, detail: &'static str) -> Result<Value, ProcedureError> {
    value
        .try_into()
        .map(Value::Int)
        .map_err(|_| ProcedureError::Internal {
            detail: format!("selene.health {detail} exceeds INTEGER range"),
        })
}
