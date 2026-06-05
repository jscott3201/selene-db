//! `selene.vector_score_candidate_state` native built-in.
//!
//! Read-only graph-tier procedure that resolves a named maintained candidate
//! set and reranks it by a vector-valued node property. The maintained set is
//! generation-checked against the statement snapshot before scoring.

use selene_core::{Value, VectorMetric};
use selene_graph::ProviderError;

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{
    BatchMismatch, cardinality_arg, invalid_arg, metric_arg, query_arg, string_arg,
    vector_search_error,
};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_score_candidate_state";

static VECTOR_SCORE_OUTPUTS: [StaticOutputColumn; 2] = [
    StaticOutputColumn::new("node_id", GqlType::NodeRef).with_description("Scored node id."),
    StaticOutputColumn::new("distance", GqlType::Float64)
        .with_description("Lower-is-better distance."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    [
        StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
        StaticParameter::new("query", GqlType::Vector, false).with_description("Query vector."),
        StaticParameter::new("state_name", GqlType::String, false)
            .with_description("Maintained candidate-state name."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum result count."),
        StaticParameter::new("metric", GqlType::String, false)
            .with_description("Distance metric.")
            .with_default_doc("squared_euclidean")
            .with_default(ProcedureDefaultValue::String("squared_euclidean")),
    ]
    .into_iter()
    .map(StaticParameter::into_parameter)
    .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    VECTOR_SCORE_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !(4..=5).contains(&args.len()) {
        return Err(invalid_arg(format!("{PROC_NAME} expects 4 or 5 arguments")));
    }

    let property = string_arg(PROC_NAME, &args[0], "property")?;
    let query = query_arg(PROC_NAME, &args[1])?;
    let state_name = string_arg(PROC_NAME, &args[2], "state_name")?;
    let k = cardinality_arg(PROC_NAME, &args[3], "k")?;
    let metric = args
        .get(4)
        .map(|arg| metric_arg(PROC_NAME, arg))
        .transpose()?
        .unwrap_or(VectorMetric::SquaredEuclidean);

    let candidates = ctx
        .vector_candidate_set(&state_name)
        .map_err(candidate_state_error)?
        .ok_or_else(|| {
            invalid_arg(format!(
                "{PROC_NAME} unknown maintained candidate-state set '{}'",
                state_name.as_str()
            ))
        })?;

    let hits = ctx
        .snapshot()
        .score_vector_candidate_set_checked(
            &property,
            &query,
            &candidates,
            metric,
            k,
            ctx.cancellation_checker(),
        )
        .map_err(|error| {
            vector_search_error(
                error,
                "maintained candidate-state vector scoring",
                BatchMismatch::Internal(
                    "maintained candidate-state scoring received batched-only error",
                ),
                "maintained candidate-state vector scoring",
            )
        })?;

    let rows = hits
        .into_iter()
        .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.distance)])
        .collect();
    Ok(ProcedureResult { rows })
}

fn candidate_state_error(error: ProviderError) -> ProcedureError {
    ProcedureError::Internal {
        detail: format!("candidate-state provider error during {PROC_NAME}: {error}"),
    }
}
