//! Implementation-defined JSON scalar functions.

use selene_core::{JsonValue, Value};

use crate::{
    SourceSpan,
    runtime::{ExecutorError, evaluator::binary_ops::string_slice},
};

use super::{
    binary_ops::{data_exception, data_exception_value, string_value},
    cast::parse_json_value,
};

pub(super) fn eval_json_parse(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&value) else {
        return data_exception("json_parse argument is not a string", span);
    };
    parse_json_value(value, span)
}

pub(super) fn eval_json_stringify(
    args: Vec<Value>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match args.into_iter().next().expect("arity checked") {
        Value::Null => Ok(Value::Null),
        Value::Json(value) => string_value(&value.to_canonical_string(), span),
        _ => data_exception("json_stringify argument is not JSON", span),
    }
}

pub(super) fn eval_json_type(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    match args.into_iter().next().expect("arity checked") {
        Value::Null => Ok(Value::Null),
        Value::Json(value) => string_value(value.json_type_name(), span),
        _ => data_exception("json_type argument is not JSON", span),
    }
}

pub(super) fn eval_json_get(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let Some(value) = select_json_value(&args, span)? else {
        return Ok(Value::Null);
    };
    JsonValue::new(value.clone())
        .map(Value::Json)
        .map_err(|err| data_exception_value(format!("selected JSON value is invalid: {err}"), span))
}

pub(super) fn eval_json_get_text(
    args: Vec<Value>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(value) = select_json_value(&args, span)? else {
        return Ok(Value::Null);
    };
    match value {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::String(value) => string_value(value, span),
        other => {
            let json = JsonValue::new(other.clone()).map_err(|err| {
                data_exception_value(format!("selected JSON value is invalid: {err}"), span)
            })?;
            string_value(&json.to_canonical_string(), span)
        }
    }
}

fn select_json_value(
    args: &[Value],
    span: SourceSpan,
) -> Result<Option<&serde_json::Value>, ExecutorError> {
    debug_assert_eq!(args.len(), 2);
    if matches!(args[0], Value::Null) || matches!(args[1], Value::Null) {
        return Ok(None);
    }
    let Value::Json(value) = &args[0] else {
        return data_exception("json_get target is not JSON", span);
    };
    match (value.as_serde(), &args[1]) {
        (serde_json::Value::Object(values), Value::String(key)) => Ok(values.get(key.as_str())),
        (serde_json::Value::Array(values), index) => {
            let Some(index) = json_array_index(index, values.len(), span)? else {
                return Ok(None);
            };
            Ok(values.get(index))
        }
        (serde_json::Value::Object(_), _) => {
            data_exception("json_get object key is not a string", span)
        }
        _ => Ok(None),
    }
}

fn json_array_index(
    value: &Value,
    len: usize,
    span: SourceSpan,
) -> Result<Option<usize>, ExecutorError> {
    match value {
        Value::Int(value) if *value >= 0 => {
            Ok(usize::try_from(*value).ok().filter(|idx| *idx < len))
        }
        Value::Int(value) => {
            let offset = value.unsigned_abs();
            let Some(offset) = usize::try_from(offset).ok() else {
                return Ok(None);
            };
            Ok((offset <= len).then_some(len - offset))
        }
        Value::Uint(value) => Ok(usize::try_from(*value).ok().filter(|idx| *idx < len)),
        _ => data_exception("json_get array index is not an integer", span),
    }
}
