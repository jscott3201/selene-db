//! Shared evaluator diagnostic helpers.

use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

/// Construct a `Value::String` from engine-produced text, mapping the IL013
/// byte-cap failure to a runtime data exception at `span`.
pub(super) fn string_value(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    match selene_core::db_string(text) {
        Ok(db_string) => Ok(Value::String(db_string)),
        Err(_err) => data_exception("string exceeds the maximum byte length", span),
    }
}

pub(super) fn data_exception<T>(
    message: impl Into<String>,
    span: SourceSpan,
) -> Result<T, ExecutorError> {
    data_exception_with(DataExceptionSubclass::InvalidValueType, message, span)
}

pub(super) fn data_exception_with<T>(
    subclass: DataExceptionSubclass,
    message: impl Into<String>,
    span: SourceSpan,
) -> Result<T, ExecutorError> {
    Err(data_exception_value_with(subclass, message, span))
}

pub(super) fn data_exception_value(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    data_exception_value_with(DataExceptionSubclass::InvalidValueType, message, span)
}

pub(super) fn data_exception_value_with(
    subclass: DataExceptionSubclass,
    message: impl Into<String>,
    span: SourceSpan,
) -> ExecutorError {
    ExecutorError::data_exception(subclass, message, span)
}
