//! Small explicit argument parsers for vector adapters.

use selene_core::{NodeId, Value};
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

pub(crate) fn required_f32_list(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Vec<f32>, ProcedureError> {
    let Value::List(values) = &args[index] else {
        return Err(invalid_argument(format!(
            "{procedure} expected {name} to be LIST<FLOAT>, got {:?}",
            args[index]
        )));
    };
    values
        .iter()
        .enumerate()
        .map(|(item_index, value)| numeric_to_f32(procedure, name, item_index, value))
        .collect()
}

pub(crate) fn required_node_ref(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<NodeId, ProcedureError> {
    match &args[index] {
        Value::NodeRef(node_id) => Ok(*node_id),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be NODE, got {other:?}"
        ))),
    }
}

pub(crate) fn required_usize(
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

pub(crate) fn nullable_option_usize(
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

pub(crate) fn nullable_node_ref_list(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Option<Vec<NodeId>>, ProcedureError> {
    match &args[index] {
        Value::Null => Ok(None),
        Value::List(values) => values
            .iter()
            .enumerate()
            .map(|(item_index, value)| match value {
                Value::NodeRef(node_id) => Ok(*node_id),
                other => Err(invalid_argument(format!(
                    "{procedure}: expected {name}[{item_index}] to be NODE, got {other:?}"
                ))),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be LIST<NODE> or NULL, got {other:?}"
        ))),
    }
}

fn numeric_to_f32(
    procedure: &'static str,
    name: &'static str,
    item_index: usize,
    value: &Value,
) -> Result<f32, ProcedureError> {
    let converted = match value {
        Value::Float(value) => *value as f32,
        Value::Float32(value) => *value,
        Value::Int(value) => *value as f32,
        Value::Uint(value) => *value as f32,
        other => {
            return Err(invalid_argument(format!(
                "{procedure} expected {name}[{item_index}] to be FLOAT, got {other:?}"
            )));
        }
    };
    Ok(converted)
}
