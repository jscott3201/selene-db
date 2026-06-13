//! Boolean operator evaluation.

use selene_core::Value;

use crate::{SourceSpan, runtime::ExecutorError};

use super::diagnostics::data_exception;

pub(super) fn eval_and(lhs: Value, rhs: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match (truth(lhs, span)?, truth(rhs, span)?) {
        (Some(false), _) | (_, Some(false)) => Ok(Value::Bool(false)),
        (Some(true), Some(true)) => Ok(Value::Bool(true)),
        _ => Ok(Value::Null),
    }
}

pub(super) fn eval_or(lhs: Value, rhs: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match (truth(lhs, span)?, truth(rhs, span)?) {
        (Some(true), _) | (_, Some(true)) => Ok(Value::Bool(true)),
        (Some(false), Some(false)) => Ok(Value::Bool(false)),
        _ => Ok(Value::Null),
    }
}

pub(super) fn eval_xor(lhs: Value, rhs: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match (truth(lhs, span)?, truth(rhs, span)?) {
        (Some(lhs), Some(rhs)) => Ok(Value::Bool(lhs ^ rhs)),
        _ => Ok(Value::Null),
    }
}

fn truth(value: Value, span: SourceSpan) -> Result<Option<bool>, ExecutorError> {
    match value {
        Value::Bool(value) => Ok(Some(value)),
        Value::Null => Ok(None),
        _ => data_exception("boolean operator operand is not boolean", span),
    }
}
