//! `selene.json_path_exists_nodes` native built-in.
//!
//! Read-only graph-tier procedure exposing exact JSON path-existence search over
//! JSON-valued node properties. The path is a JSON array of string object keys
//! and integer array indexes; this deliberately stays smaller than JSONPath.

use selene_core::Value;

use super::json_path_common::{json_search_error, path_arg};
use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{cardinality_arg, invalid_arg, string_arg};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, GraphContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const PROC_NAME: &str = "selene.json_path_exists_nodes";

static JSON_PATH_EXISTS_PARAMS: [StaticParameter; 4] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
    StaticParameter::new("path", GqlType::Json, false)
        .with_description("JSON array of string object keys and integer array indexes."),
    StaticParameter::new("k", GqlType::Integer, false).with_description("Maximum result count."),
];

static JSON_PATH_EXISTS_OUTPUTS: [StaticOutputColumn; 1] =
    [StaticOutputColumn::new("node_id", GqlType::NodeRef).with_description("Matched node id.")];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    JSON_PATH_EXISTS_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    JSON_PATH_EXISTS_OUTPUTS
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
    let path = path_arg(PROC_NAME, &args[2])?;
    let k = cardinality_arg(PROC_NAME, &args[3], "k")?;

    let hits = ctx
        .snapshot()
        .exact_json_path_exists_nodes_checked(
            &label,
            &property,
            &path,
            k,
            ctx.cancellation_checker(),
        )
        .map_err(|err| json_search_error("JSON path search", err))?;
    Ok(ProcedureResult {
        rows: hits
            .into_iter()
            .map(|hit| vec![Value::NodeRef(hit.node_id)])
            .collect(),
    })
}
