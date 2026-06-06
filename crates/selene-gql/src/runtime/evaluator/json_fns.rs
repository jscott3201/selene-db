//! Implementation-defined JSON scalar functions.

use selene_core::{JsonValue, Value, db_string};

use crate::{
    SourceSpan,
    runtime::{ExecutorError, evaluator::binary_ops::string_slice},
};

use super::{
    binary_ops::{data_exception, data_exception_value, string_value},
    cast::parse_json_value,
};

const MAX_JSON_PATH_SELECTORS: usize = 64;

pub(super) const JSON_PATH_MAX_ARGS: usize = MAX_JSON_PATH_SELECTORS + 1;

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

pub(super) fn eval_json_array_length(
    args: Vec<Value>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match args.into_iter().next().expect("arity checked") {
        Value::Null => Ok(Value::Null),
        Value::Json(value) => match value.as_serde() {
            serde_json::Value::Array(values) => Ok(Value::Int(values.len() as i64)),
            _ => data_exception("json_array_length argument is not a JSON array", span),
        },
        _ => data_exception("json_array_length argument is not JSON", span),
    }
}

pub(super) fn eval_json_object_keys(
    args: Vec<Value>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match args.into_iter().next().expect("arity checked") {
        Value::Null => Ok(Value::Null),
        Value::Json(value) => match value.as_serde() {
            serde_json::Value::Object(values) => {
                let mut keys = values.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                let mut output = Vec::with_capacity(keys.len());
                for key in keys {
                    let key = db_string(key).map_err(|err| {
                        data_exception_value(format!("JSON object key is invalid: {err}"), span)
                    })?;
                    output.push(Value::String(key));
                }
                Ok(Value::List(output))
            }
            _ => data_exception("json_object_keys argument is not a JSON object", span),
        },
        _ => data_exception("json_object_keys argument is not JSON", span),
    }
}

pub(super) fn eval_json_contains(
    args: Vec<Value>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let mut args = args.into_iter();
    let target = args.next().expect("arity checked");
    let candidate = args.next().expect("arity checked");
    if matches!(target, Value::Null) || matches!(candidate, Value::Null) {
        return Ok(Value::Null);
    }
    let Value::Json(target) = target else {
        return data_exception("json_contains target is not JSON", span);
    };
    let Value::Json(candidate) = candidate else {
        return data_exception("json_contains candidate is not JSON", span);
    };
    Ok(Value::Bool(target.contains(&candidate)))
}

pub(super) fn eval_json_get(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let Some(value) = select_json_path(&args, "json_get", span)? else {
        return Ok(Value::Null);
    };
    selected_json_value(value, span)
}

pub(super) fn eval_json_get_text(
    args: Vec<Value>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(value) = select_json_path(&args, "json_get_text", span)? else {
        return Ok(Value::Null);
    };
    selected_json_text(value, span)
}

pub(super) fn eval_json_get_path(
    args: Vec<Value>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(value) = select_json_path(&args, "json_get_path", span)? else {
        return Ok(Value::Null);
    };
    selected_json_value(value, span)
}

pub(super) fn eval_json_get_path_text(
    args: Vec<Value>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(value) = select_json_path(&args, "json_get_path_text", span)? else {
        return Ok(Value::Null);
    };
    selected_json_text(value, span)
}

pub(super) fn eval_json_has_path(
    args: Vec<Value>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match json_path_exists(&args, "json_has_path", span)? {
        Some(exists) => Ok(Value::Bool(exists)),
        None => Ok(Value::Null),
    }
}

fn selected_json_value(
    value: &serde_json::Value,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    JsonValue::new(value.clone())
        .map(Value::Json)
        .map_err(|err| data_exception_value(format!("selected JSON value is invalid: {err}"), span))
}

fn selected_json_text(value: &serde_json::Value, span: SourceSpan) -> Result<Value, ExecutorError> {
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

fn select_json_path<'a>(
    args: &'a [Value],
    function: &'static str,
    span: SourceSpan,
) -> Result<Option<&'a serde_json::Value>, ExecutorError> {
    debug_assert!(args.len() >= 2);
    debug_assert!(args.len() <= JSON_PATH_MAX_ARGS);
    if matches!(args[0], Value::Null) {
        return Ok(None);
    }
    let Value::Json(value) = &args[0] else {
        return Err(data_exception_value(
            format!("{function} target is not JSON"),
            span,
        ));
    };
    let mut current = value.as_serde();
    for selector in &args[1..] {
        if matches!(selector, Value::Null) {
            return Ok(None);
        }
        let Some(next) = select_json_child(current, selector, function, span)? else {
            return Ok(None);
        };
        current = next;
    }
    Ok(Some(current))
}

fn json_path_exists(
    args: &[Value],
    function: &'static str,
    span: SourceSpan,
) -> Result<Option<bool>, ExecutorError> {
    debug_assert!(args.len() >= 2);
    debug_assert!(args.len() <= JSON_PATH_MAX_ARGS);
    if matches!(args[0], Value::Null) {
        return Ok(None);
    }
    let Value::Json(value) = &args[0] else {
        return Err(data_exception_value(
            format!("{function} target is not JSON"),
            span,
        ));
    };
    let mut current = value.as_serde();
    for selector in &args[1..] {
        if matches!(selector, Value::Null) {
            return Ok(None);
        }
        let Some(next) = select_json_child(current, selector, function, span)? else {
            return Ok(Some(false));
        };
        current = next;
    }
    Ok(Some(true))
}

fn select_json_child<'a>(
    current: &'a serde_json::Value,
    selector: &Value,
    function: &'static str,
    span: SourceSpan,
) -> Result<Option<&'a serde_json::Value>, ExecutorError> {
    match (current, selector) {
        (serde_json::Value::Object(values), Value::String(key)) => Ok(values.get(key.as_str())),
        (serde_json::Value::Array(values), index) => {
            let Some(index) = json_array_index(index, values.len(), function, span)? else {
                return Ok(None);
            };
            Ok(values.get(index))
        }
        (serde_json::Value::Object(_), _) => Err(data_exception_value(
            format!("{function} object key is not a string"),
            span,
        )),
        _ => Ok(None),
    }
}

fn json_array_index(
    value: &Value,
    len: usize,
    function: &'static str,
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
        _ => Err(data_exception_value(
            format!("{function} array index is not an integer"),
            span,
        )),
    }
}
