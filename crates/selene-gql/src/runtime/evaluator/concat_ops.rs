//! Concatenation operator evaluation.

use std::sync::Arc;

use selene_core::{NodeId, Path, Value};

use crate::{
    ImplDefinedCaps, SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::diagnostics::{data_exception, data_exception_value, data_exception_with, string_value};

#[derive(Clone, Copy, Debug)]
pub(super) struct ConcatCaps {
    max_list_length: u32,
    max_path_length: u32,
}

impl ConcatCaps {
    pub(super) const fn from_impl_defined(caps: &ImplDefinedCaps) -> Self {
        Self {
            max_list_length: caps.max_list_length,
            max_path_length: caps.max_path_length,
        }
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
        (Value::String(lhs), Value::String(rhs)) => string_value(&format!("{lhs}{rhs}"), span),
        (Value::Bytes(lhs), Value::Bytes(rhs)) => {
            let total_len = lhs.len().checked_add(rhs.len()).ok_or_else(|| {
                data_exception_value("byte-string concatenation length overflows", span)
            })?;
            let mut value = Vec::with_capacity(total_len);
            value.extend_from_slice(&lhs);
            value.extend_from_slice(&rhs);
            Ok(Value::Bytes(Arc::<[u8]>::from(value.into_boxed_slice())))
        }
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

fn path_end_node(path: &Path) -> NodeId {
    path.segments
        .last()
        .map_or(path.start, |segment| segment.node)
}
