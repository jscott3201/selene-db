//! Shared helpers for JSON path selector-array built-ins.

use selene_core::{JsonPathSelector, Value, db_string};
use selene_graph::{GraphError, JSON_PATH_SELECTOR_LIMIT, JsonSearchError};

use super::vector_common::invalid_arg;
use crate::procedure_registry::ProcedureError;

pub(super) fn path_arg(
    proc_name: &str,
    value: &Value,
) -> Result<Vec<JsonPathSelector>, ProcedureError> {
    let Value::Json(path) = value else {
        return Err(invalid_arg(format!("{proc_name} path must be JSON")));
    };
    let serde_json::Value::Array(selectors) = path.as_serde() else {
        return Err(invalid_arg(format!(
            "{proc_name} path must be a JSON array"
        )));
    };
    if selectors.is_empty() {
        return Err(invalid_arg(format!(
            "{proc_name} path must contain at least one selector"
        )));
    }
    if selectors.len() > JSON_PATH_SELECTOR_LIMIT {
        return Err(invalid_arg(format!(
            "{proc_name} path supports at most {JSON_PATH_SELECTOR_LIMIT} selectors"
        )));
    }
    selectors
        .iter()
        .map(|selector| selector_arg(proc_name, selector))
        .collect()
}

fn selector_arg(
    proc_name: &str,
    value: &serde_json::Value,
) -> Result<JsonPathSelector, ProcedureError> {
    match value {
        serde_json::Value::String(key) => {
            let key = db_string(key).map_err(|_| {
                invalid_arg(format!("{proc_name} path object key exceeds string limits"))
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
                    "{proc_name} path array index must be an integer"
                )))
            }
        }
        _ => Err(invalid_arg(format!(
            "{proc_name} path selectors must be strings or integers"
        ))),
    }
}

pub(super) fn json_search_error(search_context: &str, error: JsonSearchError) -> ProcedureError {
    match error {
        JsonSearchError::Cancelled => ProcedureError::Cancelled,
        JsonSearchError::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
        JsonSearchError::NodeScanBudgetExceeded { limit, scanned } => {
            ProcedureError::NodeScanBudgetExceeded { limit, scanned }
        }
        JsonSearchError::Graph(GraphError::Inconsistent { reason }) => ProcedureError::Internal {
            detail: format!("graph inconsistency during {search_context}: {reason}"),
        },
        JsonSearchError::Graph(other) => ProcedureError::Internal {
            detail: format!("unexpected graph error during {search_context}: {other}"),
        },
    }
}
