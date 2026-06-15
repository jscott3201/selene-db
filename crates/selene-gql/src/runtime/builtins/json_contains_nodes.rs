//! `selene.json_contains_nodes` native built-in.
//!
//! Read-only graph-tier procedure exposing exact JSON containment over
//! JSON-valued node properties. This is a candidate-producing primitive for
//! graph/vector/text retrieval composition and deliberately stays on the native
//! `CALL selene.*` surface instead of adding JSON grammar.

use selene_core::Value;
use selene_graph::{GraphError, JsonSearchError};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{cardinality_arg, invalid_arg, string_arg};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, GraphContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const PROC_NAME: &str = "selene.json_contains_nodes";

static JSON_CONTAINS_PARAMS: [StaticParameter; 4] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
    StaticParameter::new("candidate", GqlType::Json, false)
        .with_description("JSON candidate that stored values must contain."),
    StaticParameter::new("k", GqlType::Integer, false).with_description("Maximum result count."),
];

static JSON_CONTAINS_OUTPUTS: [StaticOutputColumn; 1] =
    [StaticOutputColumn::new("node_id", GqlType::NodeRef).with_description("Matched node id.")];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    JSON_CONTAINS_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    JSON_CONTAINS_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if args.len() != 4 {
        return Err(invalid_arg(format!("{PROC_NAME} expects 4 arguments")));
    }

    let label = string_arg(PROC_NAME, &args[0], "label")?;
    let property = string_arg(PROC_NAME, &args[1], "property")?;
    let Value::Json(candidate) = &args[2] else {
        return Err(invalid_arg(format!("{PROC_NAME} candidate must be JSON")));
    };
    let k = cardinality_arg(PROC_NAME, &args[3], "k")?;

    let hits = ctx
        .snapshot()
        .exact_json_contains_nodes_checked(
            &label,
            &property,
            candidate,
            k,
            ctx.cancellation_checker(),
        )
        .map_err(json_search_error)?;
    Ok(ProcedureResult {
        rows: hits
            .into_iter()
            .map(|hit| vec![Value::NodeRef(hit.node_id)])
            .collect(),
    })
}

fn json_search_error(error: JsonSearchError) -> ProcedureError {
    match error {
        JsonSearchError::Cancelled => ProcedureError::Cancelled,
        JsonSearchError::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
        JsonSearchError::NodeScanBudgetExceeded { limit, scanned } => {
            ProcedureError::NodeScanBudgetExceeded { limit, scanned }
        }
        JsonSearchError::Graph(GraphError::Inconsistent { reason }) => ProcedureError::Internal {
            detail: format!("graph inconsistency during JSON containment search: {reason}"),
        },
        JsonSearchError::Graph(other) => ProcedureError::Internal {
            detail: format!("unexpected graph error during JSON containment search: {other}"),
        },
    }
}
