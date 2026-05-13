//! Small explicit argument parsers for adapter procedures.

use selene_core::{IStr, Value, intern_with_admission};
use selene_gql::ProcedureError;

use crate::error::invalid_argument;

pub(crate) fn expect_arity(
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

pub(crate) fn required_string(
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

pub(crate) fn nullable_istr(
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

pub(crate) fn nullable_istr_list(
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

pub(crate) fn nullable_f64(
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

pub(crate) fn nullable_usize(
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
