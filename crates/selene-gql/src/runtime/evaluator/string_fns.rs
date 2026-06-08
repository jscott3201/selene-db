//! String and adjacent scalar function evaluation.

use std::sync::Arc;

use selene_core::{Record, Value};
use unicode_normalization::UnicodeNormalization;

use crate::{
    BinaryOp, NormalForm, SourceSpan, TrimSpec, ValueExpr,
    runtime::{Binding, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError},
};

use super::{
    binary_ops::{
        data_exception, data_exception_value, data_exception_value_with, eval_equality,
        string_slice, string_value,
    },
    evaluate,
};

#[derive(Clone, Copy)]
pub(super) enum TrimSide {
    Leading,
    Trailing,
    Both,
}

impl From<TrimSpec> for TrimSide {
    fn from(value: TrimSpec) -> Self {
        match value {
            TrimSpec::Leading => Self::Leading,
            TrimSpec::Trailing => Self::Trailing,
            TrimSpec::Both => Self::Both,
        }
    }
}

pub(super) fn eval_fixed_args(
    name: &str,
    args: &[ValueExpr],
    expected: usize,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<Value>, ExecutorError> {
    eval_range_args(name, args, expected..=expected, span, binding, schema, ctx)
}

pub(super) fn eval_range_args(
    name: &str,
    args: &[ValueExpr],
    arity: std::ops::RangeInclusive<usize>,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<Value>, ExecutorError> {
    let min = *arity.start();
    let max = *arity.end();
    if args.len() < min || args.len() > max {
        return Err(ExecutorError::FunctionArityMismatch {
            name: name.to_owned(),
            expected: arity_expected(min, max),
            actual: args.len(),
            span,
        });
    }
    args.iter()
        .map(|arg| evaluate(arg, binding, schema, ctx))
        .collect()
}

fn arity_expected(min: usize, max: usize) -> &'static str {
    if min == max {
        return match min {
            0 => "0",
            1 => "1",
            2 => "2",
            3 => "3",
            _ => "fixed",
        };
    }
    match (min, max) {
        (1, 2) => "1 or 2",
        (2, 3) => "2 or 3",
        (2, 65) => "2 to 65",
        _ => "variable",
    }
}

pub(super) fn eval_length(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&value) else {
        return data_exception("length argument is not a string", span);
    };
    Ok(Value::Int(value.chars().count() as i64))
}

pub(super) fn eval_byte_length(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Value::Bytes(value) = value else {
        return data_exception("byte length argument is not a byte string", span);
    };
    Ok(Value::Int(value.len() as i64))
}

pub(super) fn eval_left_right(
    args: Vec<Value>,
    span: SourceSpan,
    from_right: bool,
) -> Result<Value, ExecutorError> {
    let source = &args[0];
    let count = &args[1];
    if matches!(source, Value::Null) || matches!(count, Value::Null) {
        return Ok(Value::Null);
    }
    let count = string_length_count(count, span)?;
    if let Some(source) = string_slice(source) {
        return eval_left_right_string(source, count, from_right, span);
    }
    if let Value::Bytes(source) = source {
        return Ok(eval_left_right_bytes(source, count, from_right));
    }
    data_exception("LEFT/RIGHT source is not a string or byte string", span)
}

fn eval_left_right_string(
    source: &str,
    count: usize,
    from_right: bool,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let chars: Vec<char> = source.chars().collect();
    let value: String = if from_right {
        let start = chars.len().saturating_sub(count);
        chars[start..].iter().copied().collect()
    } else {
        chars.iter().take(count).copied().collect()
    };
    string_value(&value, span)
}

fn eval_left_right_bytes(source: &Arc<[u8]>, count: usize, from_right: bool) -> Value {
    let slice = if from_right {
        let start = source.len().saturating_sub(count);
        &source[start..]
    } else {
        &source[..source.len().min(count)]
    };
    Value::Bytes(Arc::<[u8]>::from(slice))
}

pub(super) fn eval_multi_char_trim(
    args: Vec<Value>,
    span: SourceSpan,
    side: TrimSide,
) -> Result<Value, ExecutorError> {
    let source = &args[0];
    if matches!(source, Value::Null) {
        return Ok(Value::Null);
    }
    if args
        .get(1)
        .is_some_and(|trim_chars| matches!(trim_chars, Value::Null))
    {
        return Ok(Value::Null);
    }
    let Some(source) = string_slice(source) else {
        return data_exception("trim source is not a string", span);
    };
    let trim_chars = if let Some(trim_chars) = args.get(1) {
        string_slice(trim_chars)
            .ok_or_else(|| data_exception_value("trim characters are not a string", span))?
    } else {
        " "
    };
    string_value(&trim_by_char_set(source, trim_chars, side), span)
}

pub(super) fn eval_explicit_trim(
    source: Value,
    character: Option<Value>,
    side: TrimSide,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if matches!(source, Value::Null)
        || character
            .as_ref()
            .is_some_and(|value| matches!(value, Value::Null))
    {
        return Ok(Value::Null);
    }
    match source {
        Value::String(source) => eval_explicit_trim_string(source.as_str(), character, side, span),
        Value::Bytes(source) => eval_explicit_trim_bytes(&source, character, side, span),
        _ => data_exception("trim source is not a string or byte string", span),
    }
}

fn eval_explicit_trim_string(
    source: &str,
    character: Option<Value>,
    side: TrimSide,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let character = if let Some(character) = character {
        let Some(character) = string_slice(&character) else {
            return Err(data_exception_value_with(
                DataExceptionSubclass::ValuesNotComparable,
                "trim character is not comparable with source string",
                span,
            ));
        };
        let mut chars = character.chars();
        let Some(value) = chars.next() else {
            return Err(data_exception_value_with(
                DataExceptionSubclass::TrimError,
                "trim character must contain exactly one character",
                span,
            ));
        };
        if chars.next().is_some() {
            return Err(data_exception_value_with(
                DataExceptionSubclass::TrimError,
                "trim character must contain exactly one character",
                span,
            ));
        }
        value.to_string()
    } else {
        " ".to_owned()
    };
    string_value(&trim_by_char_set(source, &character, side), span)
}

fn eval_explicit_trim_bytes(
    source: &Arc<[u8]>,
    character: Option<Value>,
    side: TrimSide,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let trim_byte = if let Some(character) = character {
        let Value::Bytes(character) = character else {
            return Err(data_exception_value_with(
                DataExceptionSubclass::ValuesNotComparable,
                "trim character is not comparable with source byte string",
                span,
            ));
        };
        let [trim_byte] = character.as_ref() else {
            return Err(data_exception_value_with(
                DataExceptionSubclass::TrimError,
                "trim byte string must contain exactly one byte",
                span,
            ));
        };
        *trim_byte
    } else {
        b' '
    };

    let mut start = 0;
    let mut end = source.len();
    if matches!(side, TrimSide::Leading | TrimSide::Both) {
        start = source
            .iter()
            .position(|byte| *byte != trim_byte)
            .unwrap_or(source.len());
    }
    if start < end && matches!(side, TrimSide::Trailing | TrimSide::Both) {
        end = source
            .iter()
            .rposition(|byte| *byte != trim_byte)
            .map_or(start, |index| index + 1);
    }
    Ok(Value::Bytes(Arc::<[u8]>::from(&source[start..end])))
}

pub(super) fn eval_string_transform(
    args: Vec<Value>,
    span: SourceSpan,
    transform: fn(&str) -> String,
) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&value) else {
        return data_exception("string function argument is not a string", span);
    };
    string_value(&transform(value), span)
}

pub(super) fn eval_normalize(
    value: Value,
    form: Option<NormalForm>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&value) else {
        return data_exception("normalize argument is not a string", span);
    };
    string_value(
        &normalize_string(value, form.unwrap_or(NormalForm::Nfc)),
        span,
    )
}

pub(super) fn is_normalized(value: &str, form: NormalForm) -> bool {
    // Allocation-free per-form quick check — avoids materializing a normalized
    // copy per row when the input is already normalized (the common case).
    match form {
        NormalForm::Nfc => unicode_normalization::is_nfc(value),
        NormalForm::Nfd => unicode_normalization::is_nfd(value),
        NormalForm::Nfkc => unicode_normalization::is_nfkc(value),
        NormalForm::Nfkd => unicode_normalization::is_nfkd(value),
    }
}

pub(super) fn eval_trim(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&value) else {
        return data_exception("trim argument is not a string", span);
    };
    string_value(value.trim(), span)
}

pub(super) fn eval_coalesce(
    name: &str,
    args: &[ValueExpr],
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    if args.is_empty() {
        return Err(ExecutorError::FunctionArityMismatch {
            name: name.to_owned(),
            expected: "at least 1",
            actual: 0,
            span,
        });
    }
    for arg in args {
        let value = evaluate(arg, binding, schema, ctx)?;
        if !matches!(value, Value::Null) {
            return Ok(value);
        }
    }
    Ok(Value::Null)
}

pub(super) fn eval_nullif(mut args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    // Borrow both operands for the equality test; only the lhs is ever returned
    // (by value, via swap_remove), so neither clone is needed.
    let equal = eval_equality(BinaryOp::Eq, &args[0], &args[1])?;
    match equal {
        Value::Bool(true) => Ok(Value::Null),
        Value::Bool(false) | Value::Null => Ok(args.swap_remove(0)),
        _ => data_exception("nullif comparison did not produce boolean", span),
    }
}

pub(super) fn eval_size(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    match args.into_iter().next().expect("arity checked") {
        Value::Null => Ok(Value::Null),
        Value::List(values) => Ok(Value::Int(values.len() as i64)),
        Value::Record(record) => {
            let len = match *record {
                Record::Open(fields) => fields.len(),
                _ => return data_exception("unsupported record shape", span),
            };
            Ok(Value::Int(len as i64))
        }
        Value::RecordTyped(record) => Ok(Value::Int(record.values.len() as i64)),
        value => {
            let Some(value) = string_slice(&value) else {
                return data_exception("size argument is not a list, record, or string", span);
            };
            Ok(Value::Int(value.chars().count() as i64))
        }
    }
}

fn normalize_string(value: &str, form: NormalForm) -> String {
    match form {
        NormalForm::Nfc => value.nfc().collect(),
        NormalForm::Nfd => value.nfd().collect(),
        NormalForm::Nfkc => value.nfkc().collect(),
        NormalForm::Nfkd => value.nfkd().collect(),
    }
}

fn string_length_count(value: &Value, span: SourceSpan) -> Result<usize, ExecutorError> {
    match value {
        Value::Int(value) if *value >= 0 => Ok(*value as usize),
        Value::Int(_) => Err(data_exception_value_with(
            DataExceptionSubclass::SubstringError,
            "string length is negative",
            span,
        )),
        Value::Uint(value) => usize::try_from(*value).map_err(|_| {
            data_exception_value_with(
                DataExceptionSubclass::NumericValueOutOfRange,
                "integer argument is too large",
                span,
            )
        }),
        _ => data_exception("string length is not an integer", span),
    }
}

fn trim_by_char_set(source: &str, trim_chars: &str, side: TrimSide) -> String {
    let chars: Vec<char> = source.chars().collect();
    let trims = |candidate: char| trim_chars.chars().any(|trim| trim == candidate);
    let start = if matches!(side, TrimSide::Leading | TrimSide::Both) {
        chars
            .iter()
            .position(|candidate| !trims(*candidate))
            .unwrap_or(chars.len())
    } else {
        0
    };
    let end = if matches!(side, TrimSide::Trailing | TrimSide::Both) {
        chars
            .iter()
            .rposition(|candidate| !trims(*candidate))
            .map_or(start, |index| index.saturating_add(1))
    } else {
        chars.len()
    };
    chars[start..end].iter().copied().collect()
}
