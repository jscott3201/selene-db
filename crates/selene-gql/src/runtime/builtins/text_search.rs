//! `selene.text_search_nodes` and `selene.text_score_nodes` native built-ins.
//!
//! Read-only graph-tier procedure exposing BM25 full-text search over
//! string-valued node properties. A registered maintained text index is used
//! when present; otherwise global search falls back to the exact scan oracle.
//! Candidate-scoped scoring requires a registered text index so a read call does
//! not unexpectedly build a full transient postings index. These are
//! intentionally `CALL selene.*` surfaces rather than grammar syntax: full-text
//! search is an implementation-defined engine capability layered on ISO GQL
//! values.

use selene_core::Value;
use selene_graph::{GraphError, TextSearchError};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{cardinality_arg, invalid_arg, node_list_arg, string_arg};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, GraphContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const PROC_NAME: &str = "selene.text_search_nodes";
const SCORE_PROC_NAME: &str = "selene.text_score_nodes";

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

pub(super) fn score_signature() -> Vec<ProcedureParameter> {
    [
        StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
        StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
        StaticParameter::new("query", GqlType::String, false)
            .with_description("Full-text query string."),
        StaticParameter::new("nodes", GqlType::List(Box::new(GqlType::NodeRef)), false)
            .with_description("Candidate nodes to score."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum result count."),
    ]
    .into_iter()
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
    let query = query_arg(PROC_NAME, &args[2])?;
    let k = cardinality_arg(PROC_NAME, &args[3], "k")?;

    let snapshot = ctx.snapshot();
    let hits = match snapshot.text_index_for(&label, &property) {
        Some(index) => index
            .search_checked(query, k, ctx.cancellation_checker())
            .map_err(text_search_error)?,
        None => snapshot
            .exact_text_search_nodes_checked(
                &label,
                &property,
                query,
                k,
                ctx.cancellation_checker(),
            )
            .map_err(text_search_error)?,
    };
    Ok(ProcedureResult {
        rows: hits
            .into_iter()
            .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.score)])
            .collect(),
    })
}

pub(super) fn execute_score(
    ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if args.len() != 5 {
        return Err(invalid_arg(format!(
            "{SCORE_PROC_NAME} expects 5 arguments"
        )));
    }

    let label = string_arg(SCORE_PROC_NAME, &args[0], "label")?;
    let property = string_arg(SCORE_PROC_NAME, &args[1], "property")?;
    let query = query_arg(SCORE_PROC_NAME, &args[2])?;
    let nodes = node_list_arg(SCORE_PROC_NAME, &args[3], "nodes")?;
    let k = cardinality_arg(SCORE_PROC_NAME, &args[4], "k")?;

    let snapshot = ctx.snapshot();
    let Some(index) = snapshot.text_index_for(&label, &property) else {
        return Err(invalid_arg(format!(
            "{SCORE_PROC_NAME} requires a text index for {}.{}; call selene.create_text_index first",
            label.as_str(),
            property.as_str()
        )));
    };
    let hits = index
        .search_candidates_checked(query, &nodes, k, ctx.cancellation_checker())
        .map_err(text_search_error)?;

    Ok(ProcedureResult {
        rows: hits
            .into_iter()
            .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.score)])
            .collect(),
    })
}

fn query_arg<'a>(proc_name: &'static str, value: &'a Value) -> Result<&'a str, ProcedureError> {
    let Value::String(value) = value else {
        return Err(invalid_arg(format!("{proc_name} query must be a STRING")));
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
