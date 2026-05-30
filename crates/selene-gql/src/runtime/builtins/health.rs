//! `selene.health` native built-in.
//!
//! Read-only graph-tier procedure reporting basic graph health counters. Ported
//! verbatim from `selene-pack/src/builtin/health.rs`; the only change is that
//! metadata is produced through the shared `Static*` → planner-metadata helpers
//! and execution takes a [`GraphContext`] directly.

use selene_core::Value;

use super::meta::{StaticOutputColumn, StaticParameter};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, GraphContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

static HEALTH_OUTPUTS: [StaticOutputColumn; 4] = [
    StaticOutputColumn::new("graph_id", GqlType::Uint64).with_description("Graph identifier."),
    StaticOutputColumn::new("node_count", GqlType::Uint64).with_description("Node count."),
    StaticOutputColumn::new("edge_count", GqlType::Uint64).with_description("Edge count."),
    StaticOutputColumn::new("schema_bound", GqlType::Boolean)
        .with_description("Whether a graph type is bound."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    let params: [StaticParameter; 0] = [];
    params
        .into_iter()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    HEALTH_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
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
