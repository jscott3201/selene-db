//! `selene.vector_candidate_states` native built-in.
//!
//! Read-only graph-tier procedure exposing generation-checked metadata for
//! maintained vector candidate-state sets.

use selene_core::{DbString, Value};
use selene_graph::{ProviderError, VectorCandidateStateInfo};

use super::meta::{StaticOutputColumn, StaticParameter};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, GraphContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const PROC_NAME: &str = "selene.vector_candidate_states";

pub(super) fn signature() -> Vec<ProcedureParameter> {
    let params: [StaticParameter; 0] = [];
    params
        .into_iter()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    [
        StaticOutputColumn::new("state_name", GqlType::String)
            .with_description("Maintained candidate-state name."),
        StaticOutputColumn::new("generation", GqlType::Uint64)
            .with_description("Graph generation covered by this descriptor."),
        StaticOutputColumn::new("candidate_count", GqlType::Uint64)
            .with_description("Current candidate node count."),
        StaticOutputColumn::new("required_label", GqlType::String)
            .with_nullable(true)
            .with_description("Required node label, or NULL when unconstrained."),
        StaticOutputColumn::new("require_outgoing", GqlType::List(Box::new(GqlType::String)))
            .with_description("Outgoing edge labels required on the source node."),
        StaticOutputColumn::new("require_incoming", GqlType::List(Box::new(GqlType::String)))
            .with_description("Incoming edge labels required on the target node."),
        StaticOutputColumn::new("exclude_outgoing", GqlType::List(Box::new(GqlType::String)))
            .with_description("Outgoing edge labels that disqualify the source node."),
        StaticOutputColumn::new("exclude_incoming", GqlType::List(Box::new(GqlType::String)))
            .with_description("Incoming edge labels that disqualify the target node."),
    ]
    .into_iter()
    .map(StaticOutputColumn::into_output_column)
    .collect()
}

pub(super) fn execute(
    ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !args.is_empty() {
        return Err(ProcedureError::InvalidArgument {
            detail: format!("{PROC_NAME} expects zero arguments"),
        });
    }

    let mut infos = ctx
        .vector_candidate_state_infos()
        .map_err(candidate_state_error)?;
    infos.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
    let rows = infos.into_iter().map(info_into_values).collect();
    Ok(ProcedureResult { rows })
}

fn info_into_values(info: VectorCandidateStateInfo) -> Vec<Value> {
    vec![
        Value::String(info.name),
        Value::Uint(info.generation),
        Value::Uint(usize_to_u64_saturating(info.candidate_count)),
        optional_string(info.required_label),
        string_list(info.require_outgoing),
        string_list(info.require_incoming),
        string_list(info.exclude_outgoing),
        string_list(info.exclude_incoming),
    ]
}

fn optional_string(value: Option<DbString>) -> Value {
    value.map_or(Value::Null, Value::String)
}

fn string_list(values: Vec<DbString>) -> Value {
    Value::List(values.into_iter().map(Value::String).collect())
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn candidate_state_error(error: ProviderError) -> ProcedureError {
    ProcedureError::Internal {
        detail: format!("candidate-state provider error during {PROC_NAME}: {error}"),
    }
}
