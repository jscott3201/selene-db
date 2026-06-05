//! `selene.text_search_nodes` native built-in.
//!
//! Read-only graph-tier procedure exposing the exact BM25 full-text oracle over
//! string-valued node properties. This is intentionally a `CALL selene.*`
//! surface rather than grammar syntax: full-text search is an
//! implementation-defined engine capability layered on ISO GQL values.

use selene_core::Value;
use selene_graph::{GraphError, TextSearchError};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{cardinality_arg, invalid_arg, string_arg};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, GraphContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const PROC_NAME: &str = "selene.text_search_nodes";

static TEXT_SEARCH_PARAMS: [StaticParameter; 4] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
    StaticParameter::new("query", GqlType::String, false)
        .with_description("Full-text query string."),
    StaticParameter::new("k", GqlType::Integer, false).with_description("Maximum result count."),
];

static TEXT_SEARCH_OUTPUTS: [StaticOutputColumn; 2] = [
    StaticOutputColumn::new("node_id", GqlType::NodeRef).with_description("Matched node id."),
    StaticOutputColumn::new("score", GqlType::Float64)
        .with_description("Higher-is-better BM25 score."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    TEXT_SEARCH_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    TEXT_SEARCH_OUTPUTS
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
    let query = query_arg(&args[2])?;
    let k = cardinality_arg(PROC_NAME, &args[3], "k")?;

    let hits = ctx
        .snapshot()
        .exact_text_search_nodes_checked(&label, &property, query, k, ctx.cancellation_checker())
        .map_err(text_search_error)?;
    Ok(ProcedureResult {
        rows: hits
            .into_iter()
            .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.score)])
            .collect(),
    })
}

fn query_arg(value: &Value) -> Result<&str, ProcedureError> {
    let Value::String(value) = value else {
        return Err(invalid_arg(format!("{PROC_NAME} query must be a STRING")));
    };
    Ok(value.as_str())
}

fn text_search_error(error: TextSearchError) -> ProcedureError {
    match error {
        TextSearchError::Cancelled => ProcedureError::Cancelled,
        TextSearchError::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
        TextSearchError::Graph(GraphError::Inconsistent { reason }) => ProcedureError::Internal {
            detail: format!("graph inconsistency during text search: {reason}"),
        },
        TextSearchError::Graph(other) => ProcedureError::Internal {
            detail: format!("unexpected graph error during text search: {other}"),
        },
    }
}
