//! Concatenation operator evaluation.

use std::sync::Arc;

use selene_core::{NodeId, Path, Value};
use unicode_normalization::UnicodeNormalization;

use crate::{
    ImplDefinedCaps, SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::diagnostics::{data_exception, data_exception_with};

#[derive(Clone, Copy, Debug)]
pub(super) struct ConcatCaps {
    max_string_length: u32,
    max_byte_string_length: u32,
    max_list_length: u32,
    max_path_length: u32,
}

impl ConcatCaps {
    pub(super) const fn from_impl_defined(caps: &ImplDefinedCaps) -> Self {
        Self {
            max_string_length: caps.max_string_length,
            max_byte_string_length: caps.max_byte_string_length,
            max_list_length: caps.max_list_length,
            max_path_length: caps.max_path_length,
        }
    }

    fn max_string_length(self) -> usize {
        usize::try_from(self.max_string_length).unwrap_or(usize::MAX)
    }

    fn max_byte_string_length(self) -> usize {
        usize::try_from(self.max_byte_string_length).unwrap_or(usize::MAX)
    }

    fn max_list_length(self) -> usize {
        usize::try_from(self.max_list_length).unwrap_or(usize::MAX)
    }
}

pub(super) fn eval_concat(
    lhs: Value,
    rhs: Value,
    span: SourceSpan,
    caps: ConcatCaps,
) -> Result<Value, ExecutorError> {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return Ok(Value::Null);
    }
    match (lhs, rhs) {
        (Value::String(lhs), Value::String(rhs)) => {
            concat_strings(lhs.as_str(), rhs.as_str(), span, caps)
        }
        (Value::Bytes(lhs), Value::Bytes(rhs)) => concat_bytes(&lhs, &rhs, span, caps),
        (Value::List(mut lhs), Value::List(rhs)) => {
            let total_len = lhs.len().checked_add(rhs.len()).ok_or_else(|| {
                ExecutorError::data_exception(
                    DataExceptionSubclass::ListDataRightTruncation,
                    "list concatenation length overflows",
                    span,
                )
            })?;
            if total_len > caps.max_list_length() {
                return data_exception_with(
                    DataExceptionSubclass::ListDataRightTruncation,
                    "list concatenation exceeds the configured maximum list cardinality",
                    span,
                );
            }
            lhs.extend(rhs);
            Ok(Value::List(lhs))
        }
        (Value::Path(lhs), Value::Path(rhs)) => {
            concat_paths(*lhs, *rhs, span, caps.max_path_length)
        }
        _ => data_exception(
            "concatenation operands must both be strings, byte strings, lists, or paths",
            span,
        ),
    }
}

fn concat_strings(
    lhs: &str,
    rhs: &str,
    span: SourceSpan,
    caps: ConcatCaps,
) -> Result<Value, ExecutorError> {
    let byte_len = lhs.len().checked_add(rhs.len()).ok_or_else(|| {
        string_truncation_value("character-string concatenation length overflows", span)
    })?;
    let mut value = String::with_capacity(byte_len);
    value.push_str(lhs);
    value.push_str(rhs);
    if unicode_normalization::is_nfc(lhs) && unicode_normalization::is_nfc(rhs) {
        value = value.nfc().collect();
    }
    let char_count = value.chars().count();
    let max_chars = caps.max_string_length();
    if char_count <= max_chars {
        return string_concat_value(&value, span);
    }
    let overflow_chars = char_count - max_chars;
    if !value
        .chars()
        .rev()
        .take(overflow_chars)
        .all(char::is_whitespace)
    {
        return string_truncation(
            "character-string concatenation exceeds the configured maximum length",
            span,
        );
    }
    string_concat_value(&value.chars().take(max_chars).collect::<String>(), span)
}

fn concat_bytes(
    lhs: &[u8],
    rhs: &[u8],
    span: SourceSpan,
    caps: ConcatCaps,
) -> Result<Value, ExecutorError> {
    let total_len = lhs.len().checked_add(rhs.len()).ok_or_else(|| {
        string_truncation_value("byte-string concatenation length overflows", span)
    })?;
    let output_len =
        capped_byte_concat_len(lhs, rhs, total_len, caps.max_byte_string_length(), span)?;
    let mut value = Vec::with_capacity(output_len);
    if output_len <= lhs.len() {
        value.extend_from_slice(&lhs[..output_len]);
    } else {
        value.extend_from_slice(lhs);
        value.extend_from_slice(&rhs[..output_len - lhs.len()]);
    }
    Ok(Value::Bytes(Arc::<[u8]>::from(value.into_boxed_slice())))
}

fn capped_byte_concat_len(
    lhs: &[u8],
    rhs: &[u8],
    total_len: usize,
    max_len: usize,
    span: SourceSpan,
) -> Result<usize, ExecutorError> {
    if total_len <= max_len {
        return Ok(total_len);
    }
    let overflow = total_len - max_len;
    if !byte_suffix_is_zero(lhs, rhs, overflow) {
        return string_truncation(
            "byte-string concatenation exceeds the configured maximum length",
            span,
        );
    }
    Ok(max_len)
}

fn byte_suffix_is_zero(lhs: &[u8], rhs: &[u8], suffix_len: usize) -> bool {
    if suffix_len <= rhs.len() {
        return rhs[rhs.len() - suffix_len..].iter().all(|byte| *byte == 0);
    }
    rhs.iter().all(|byte| *byte == 0)
        && lhs[lhs.len() - (suffix_len - rhs.len())..]
            .iter()
            .all(|byte| *byte == 0)
}

fn concat_paths(
    mut lhs: Path,
    rhs: Path,
    span: SourceSpan,
    max_path_length: u32,
) -> Result<Value, ExecutorError> {
    if lhs.graph != rhs.graph || path_end_node(&lhs) != rhs.start {
        return data_exception_with(
            DataExceptionSubclass::MalformedPath,
            "path concatenation endpoints do not identify the same node",
            span,
        );
    }
    let segment_count = lhs
        .segments
        .len()
        .checked_add(rhs.segments.len())
        .ok_or_else(|| {
            ExecutorError::data_exception(
                DataExceptionSubclass::PathDataRightTruncation,
                "path concatenation length overflows",
                span,
            )
        })?;
    let max_path_length = usize::try_from(max_path_length).unwrap_or(usize::MAX);
    if segment_count > max_path_length {
        return data_exception_with(
            DataExceptionSubclass::PathDataRightTruncation,
            "path concatenation exceeds the configured maximum path length",
            span,
        );
    }
    lhs.segments.extend(rhs.segments);
    Ok(Value::Path(Box::new(lhs)))
}

fn string_truncation<T>(message: impl Into<String>, span: SourceSpan) -> Result<T, ExecutorError> {
    data_exception_with(
        DataExceptionSubclass::StringDataRightTruncation,
        message,
        span,
    )
}

fn string_truncation_value(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::StringDataRightTruncation,
        message,
        span,
    )
}

fn string_concat_value(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    selene_core::db_string(text)
        .map(Value::String)
        .map_err(|_err| string_truncation_value("character-string concatenation is too long", span))
}

fn path_end_node(path: &Path) -> NodeId {
    path.segments
        .last()
        .map_or(path.start, |segment| segment.node)
}
