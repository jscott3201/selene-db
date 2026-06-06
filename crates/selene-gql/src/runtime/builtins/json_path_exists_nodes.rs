//! `selene.json_path_exists_nodes` native built-in.
//!
//! Read-only graph-tier procedure exposing exact JSON path-existence search over
//! JSON-valued node properties. The path is a JSON array of string object keys
//! and integer array indexes; this deliberately stays smaller than JSONPath.

use selene_core::{JsonPathSelector, Value, db_string};
use selene_graph::{GraphError, JSON_PATH_SELECTOR_LIMIT, JsonSearchError};

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
    let path = path_arg(&args[2])?;
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
        .map_err(json_search_error)?;
    Ok(ProcedureResult {
        rows: hits
            .into_iter()
            .map(|hit| vec![Value::NodeRef(hit.node_id)])
            .collect(),
    })
}

fn path_arg(value: &Value) -> Result<Vec<JsonPathSelector>, ProcedureError> {
    let Value::Json(path) = value else {
        return Err(invalid_arg(format!("{PROC_NAME} path must be JSON")));
    };
    let serde_json::Value::Array(selectors) = path.as_serde() else {
        return Err(invalid_arg(format!(
            "{PROC_NAME} path must be a JSON array"
        )));
    };
    if selectors.is_empty() {
        return Err(invalid_arg(format!(
            "{PROC_NAME} path must contain at least one selector"
        )));
    }
    if selectors.len() > JSON_PATH_SELECTOR_LIMIT {
        return Err(invalid_arg(format!(
            "{PROC_NAME} path supports at most {JSON_PATH_SELECTOR_LIMIT} selectors"
        )));
    }
    selectors.iter().map(selector_arg).collect()
}

fn selector_arg(value: &serde_json::Value) -> Result<JsonPathSelector, ProcedureError> {
    match value {
        serde_json::Value::String(key) => {
            let key = db_string(key).map_err(|_| {
                invalid_arg(format!("{PROC_NAME} path object key exceeds string limits"))
            })?;
            Ok(JsonPathSelector::Key(key))
        }
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(JsonPathSelector::Index(value))
            } else if let Some(value) = number.as_u64() {
                Ok(JsonPathSelector::UnsignedIndex(value))
            } else {
                Err(invalid_arg(format!(
                    "{PROC_NAME} path array index must be an integer"
                )))
            }
        }
        _ => Err(invalid_arg(format!(
            "{PROC_NAME} path selectors must be strings or integers"
        ))),
    }
}

fn json_search_error(error: JsonSearchError) -> ProcedureError {
    match error {
        JsonSearchError::Cancelled => ProcedureError::Cancelled,
        JsonSearchError::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
        JsonSearchError::Graph(GraphError::Inconsistent { reason }) => ProcedureError::Internal {
            detail: format!("graph inconsistency during JSON path search: {reason}"),
        },
        JsonSearchError::Graph(other) => ProcedureError::Internal {
            detail: format!("unexpected graph error during JSON path search: {other}"),
        },
    }
}
