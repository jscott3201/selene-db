//! Small explicit argument parsers for the native `algo.*` procedures.
//!
//! Ported verbatim from the historical procedure-pack argument adapter.
//! These parsers **define** the procedure signatures: their coercion
//! and arity rules (Int→f64, NULL→default, `nullable_*`, trailing-nullable
//! arity, negative-int rejection, overflow checks) are the user-visible
//! contract. Any drift here silently changes a procedure's accepted arguments,
//! so the edge-case tests are ported verbatim alongside them.

use selene_core::{IStr, NodeId, Value, intern_with_admission};

use super::error::invalid_argument;
use crate::ProcedureError;

pub(super) fn expect_arity(
    procedure: &'static str,
    args: &[Value],
    expected: usize,
) -> Result<(), ProcedureError> {
    if args.len() == expected {
        return Ok(());
    }
    Err(invalid_argument(format!(
        "{procedure} expected {expected} arguments, got {}",
        args.len()
    )))
}

pub(super) fn required_string(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<String, ProcedureError> {
    match &args[index] {
        Value::String(value) => Ok(value.as_str().to_owned()),
        Value::ExternalString(value) => Ok(value.to_string()),
        other => Err(invalid_argument(format!(
            "{procedure} expected {name} to be STRING, got {other:?}"
        ))),
    }
}

pub(super) fn required_node_ref(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<NodeId, ProcedureError> {
    match &args[index] {
        Value::NodeRef(value) => Ok(*value),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be NODE, got {other:?}"
        ))),
    }
}

pub(super) fn nullable_istr(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Option<IStr>, ProcedureError> {
    match &args[index] {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(*value)),
        Value::ExternalString(value) => Ok(Some(intern_label(procedure, name, value)?)),
        other => Err(invalid_argument(format!(
            "{procedure} expected {name} to be STRING or NULL, got {other:?}"
        ))),
    }
}

pub(super) fn nullable_istr_list(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Vec<IStr>, ProcedureError> {
    match &args[index] {
        Value::Null => Ok(Vec::new()),
        Value::List(values) => values
            .iter()
            .enumerate()
            .map(|(item_index, value)| match value {
                Value::String(value) => Ok(*value),
                Value::ExternalString(value) => intern_label(procedure, name, value),
                other => Err(invalid_argument(format!(
                    "{procedure} expected {name}[{item_index}] to be STRING, got {other:?}"
                ))),
            })
            .collect(),
        other => Err(invalid_argument(format!(
            "{procedure} expected {name} to be LIST<STRING> or NULL, got {other:?}"
        ))),
    }
}

pub(super) fn nullable_f64(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
    default: f64,
) -> Result<f64, ProcedureError> {
    match &args[index] {
        Value::Null => Ok(default),
        Value::Float(value) => Ok(*value),
        Value::Float32(value) => Ok(f64::from(*value)),
        Value::Int(value) => Ok(*value as f64),
        Value::Uint(value) => Ok(*value as f64),
        other => Err(invalid_argument(format!(
            "{procedure} expected {name} to be FLOAT or NULL, got {other:?}"
        ))),
    }
}

pub(super) fn nullable_usize(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
    default: usize,
) -> Result<usize, ProcedureError> {
    match &args[index] {
        Value::Null => Ok(default),
        Value::Int(value) if *value >= 0 => usize::try_from(*value)
            .map_err(|_| invalid_argument(format!("{procedure} {name} is too large"))),
        Value::Int(_) => Err(invalid_argument(format!(
            "{procedure} {name} must be non-negative"
        ))),
        Value::Uint(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument(format!("{procedure} {name} is too large"))),
        other => Err(invalid_argument(format!(
            "{procedure} expected {name} to be INTEGER or NULL, got {other:?}"
        ))),
    }
}

pub(super) fn nullable_option_usize(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Option<usize>, ProcedureError> {
    match &args[index] {
        Value::Null => Ok(None),
        Value::Int(value) if *value >= 0 => usize::try_from(*value)
            .map(Some)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        Value::Int(_) => Err(invalid_argument(format!(
            "{procedure}: {name} must be non-negative"
        ))),
        Value::Uint(value) => usize::try_from(*value)
            .map(Some)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be INTEGER or NULL, got {other:?}"
        ))),
    }
}

pub(super) fn required_nonnegative_usize(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<usize, ProcedureError> {
    match &args[index] {
        Value::Int(value) if *value >= 0 => usize::try_from(*value)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        Value::Int(_) => Err(invalid_argument(format!(
            "{procedure}: {name} must be non-negative"
        ))),
        Value::Uint(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be INTEGER, got {other:?}"
        ))),
    }
}

fn intern_label(
    procedure: &'static str,
    name: &'static str,
    value: &str,
) -> Result<IStr, ProcedureError> {
    intern_with_admission(value)
        .map(|(istr, _was_new)| istr)
        .map_err(|_error| {
            invalid_argument(format!(
                "{procedure} could not intern {name} value {value:?}"
            ))
        })
}
