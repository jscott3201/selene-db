//! Implementation-defined UUID scalar functions.

use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{ExecutorError, evaluator::binary_ops::string_slice},
};

use super::binary_ops::{data_exception, data_exception_value};

pub(super) fn eval_uuid_v4(_args: Vec<Value>) -> Result<Value, ExecutorError> {
    Ok(Value::Uuid(uuid::Uuid::new_v4()))
}

pub(super) fn eval_uuid_v7(_args: Vec<Value>) -> Result<Value, ExecutorError> {
    Ok(Value::Uuid(uuid::Uuid::now_v7()))
}

pub(super) fn eval_uuid(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&value) else {
        return data_exception("uuid argument is not a string", span);
    };
    parse_uuid_string(value, span).map(Value::Uuid)
}

pub(super) fn parse_uuid_string(
    value: &str,
    span: SourceSpan,
) -> Result<uuid::Uuid, ExecutorError> {
    uuid::Uuid::parse_str(value).map_err(|_| data_exception_value("invalid UUID string", span))
}
