//! String and adjacent scalar function evaluation.

use std::sync::Arc;

use selene_core::{Record, Value};

use crate::{
    BinaryOp, SourceSpan, ValueExpr,
    runtime::{Binding, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError},
};

use super::{
    binary_ops::{
        data_exception, data_exception_value, data_exception_value_with, eval_equality,
        string_slice,
    },
    evaluate,
};

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
            expected: if min == max {
                match min {
                    1 => "1",
                    2 => "2",
                    _ => "fixed",
                }
            } else {
                "2 or 3"
            },
            actual: args.len(),
            span,
        });
    }
    args.iter()
        .map(|arg| evaluate(arg, binding, schema, ctx))
        .collect()
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

pub(super) fn eval_substring(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let source = &args[0];
    if matches!(source, Value::Null) {
        return Ok(Value::Null);
    }
    if matches!(&args[1], Value::Null) {
        return Ok(Value::Null);
    }
    if args
        .get(2)
        .is_some_and(|length| matches!(length, Value::Null))
    {
        return Ok(Value::Null);
    }
    let Some(source) = string_slice(source) else {
        return data_exception("substring source is not a string", span);
    };
    let start = non_negative_usize(
        &args[1],
        "substring start is not a non-negative integer",
        span,
    )?;
    let length = if let Some(length) = args.get(2) {
        Some(non_negative_usize(
            length,
            "substring length is not a non-negative integer",
            span,
        )?)
    } else {
        None
    };
    let chars: Vec<char> = source.chars().collect();
    if start >= chars.len() {
        return Ok(Value::ExternalString(Arc::from("")));
    }
    let end = length.map_or(chars.len(), |length| {
        start.saturating_add(length).min(chars.len())
    });
    let value: String = chars[start..end].iter().collect();
    Ok(Value::ExternalString(Arc::from(value)))
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
    Ok(Value::ExternalString(Arc::from(transform(value))))
}

pub(super) fn eval_trim(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&value) else {
        return data_exception("trim argument is not a string", span);
    };
    Ok(Value::ExternalString(Arc::from(value.trim())))
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

pub(super) fn eval_nullif(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let lhs = args[0].clone();
    let rhs = args[1].clone();
    match eval_equality(BinaryOp::Eq, &lhs, &rhs)? {
        Value::Bool(true) => Ok(Value::Null),
        Value::Bool(false) | Value::Null => Ok(lhs),
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

fn non_negative_usize(
    value: &Value,
    message: &'static str,
    span: SourceSpan,
) -> Result<usize, ExecutorError> {
    match value {
        Value::Int(value) if *value >= 0 => Ok(*value as usize),
        Value::Uint(value) => usize::try_from(*value).map_err(|_| {
            data_exception_value_with(
                DataExceptionSubclass::NumericValueOutOfRange,
                "integer argument is too large",
                span,
            )
        }),
        Value::Null => Err(data_exception_value(message, span)),
        _ => data_exception(message, span),
    }
}
