//! Scalar function evaluation.
//!
//! Dispatches the v1.1 closed scalar-function set case-insensitively:
//! `abs`, `ceil`, `floor`, `round`, `mod`, `sqrt`, `power`, trigonometric and
//! logarithmic functions, `element_id`, `cardinality`, `length`, `char_length`,
//! `character_length`, `substring`, `upper`, `lower`, `trim`, `coalesce`,
//! `nullif`, and `size`.
//! Each function owns arity checking; `NULL` propagates except where
//! short-circuit functions (`coalesce`, `nullif`) define different behavior.

use std::sync::Arc;

use selene_core::{IStr, Record, Value};

use crate::{
    BinaryOp, NonEmpty, SourceSpan, ValueExpr,
    runtime::{Binding, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError},
};

use super::{
    binary_ops::{
        data_exception, data_exception_value, data_exception_value_with, data_exception_with,
        eval_binary, eval_equality, numeric_to_f64, string_slice,
    },
    evaluate, identity_length_fns,
};

pub(super) fn eval_function_call(
    name: &NonEmpty<IStr>,
    args: &[ValueExpr],
    (star, distinct): (bool, bool),
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let display_name = display_name(name);
    if star {
        return Err(ExecutorError::InvalidFunctionModifier {
            name: display_name,
            modifier: "*",
            span,
        });
    }
    if distinct {
        return Err(ExecutorError::InvalidFunctionModifier {
            name: display_name,
            modifier: "DISTINCT",
            span,
        });
    }
    let Some(segment) = single_segment_name(name) else {
        return Err(ExecutorError::UnknownFunction {
            name: display_name,
            span,
        });
    };

    match segment.as_str() {
        "abs" => eval_abs(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "ceil" | "ceiling" => eval_unary_numeric(
            &display_name,
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::ceil,
        ),
        "floor" => eval_unary_numeric(
            &display_name,
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::floor,
        ),
        "sin" => eval_unary_float(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::sin,
            "trigonometric result is non-finite",
        ),
        "cos" => eval_unary_float(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::cos,
            "trigonometric result is non-finite",
        ),
        "tan" => eval_unary_float(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::tan,
            "trigonometric result is non-finite",
        ),
        "cot" => eval_cot(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "sinh" => eval_unary_float(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::sinh,
            "trigonometric result is non-finite",
        ),
        "cosh" => eval_unary_float(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::cosh,
            "trigonometric result is non-finite",
        ),
        "tanh" => eval_unary_float(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::tanh,
            "trigonometric result is non-finite",
        ),
        "asin" => eval_inverse_trig(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::asin,
        ),
        "acos" => eval_inverse_trig(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::acos,
        ),
        "atan" => eval_unary_float(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::atan,
            "trigonometric result is non-finite",
        ),
        "degrees" => eval_unary_float(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::to_degrees,
            "degree conversion result is non-finite",
        ),
        "radians" => eval_unary_float(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::to_radians,
            "radian conversion result is non-finite",
        ),
        "ln" => eval_ln(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "log" => eval_log(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
        ),
        "log10" => eval_log10(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "exp" => eval_exp(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "round" => eval_unary_numeric(
            &display_name,
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::round,
        ),
        "mod" => {
            let args = eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?;
            eval_binary(BinaryOp::Mod, args[0].clone(), args[1].clone(), span)
        }
        "sqrt" => eval_sqrt(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "power" => {
            let args = eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?;
            eval_binary(BinaryOp::Power, args[0].clone(), args[1].clone(), span)
        }
        "element_id" => identity_length_fns::eval_element_id(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "cardinality" => identity_length_fns::eval_cardinality(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            ctx,
        ),
        "length" | "char_length" | "character_length" => eval_length(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "substring" => eval_substring(
            eval_range_args(&display_name, args, 2..=3, span, binding, schema, ctx)?,
            span,
        ),
        "upper" => eval_string_transform(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            str::to_uppercase,
        ),
        "lower" => eval_string_transform(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            str::to_lowercase,
        ),
        "trim" => eval_trim(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "coalesce" => eval_coalesce(&display_name, args, span, binding, schema, ctx),
        "nullif" => eval_nullif(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
        ),
        "size" => eval_size(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        _ => Err(ExecutorError::UnknownFunction {
            name: display_name,
            span,
        }),
    }
}

fn eval_abs(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    match args.into_iter().next().expect("arity checked") {
        Value::Null => Ok(Value::Null),
        Value::Int(value) => value.checked_abs().map(Value::Int).ok_or_else(|| {
            data_exception_value_with(
                DataExceptionSubclass::NumericValueOutOfRange,
                "integer absolute value overflow",
                span,
            )
        }),
        Value::Int128(value) => value.checked_abs().map(Value::Int128).ok_or_else(|| {
            data_exception_value_with(
                DataExceptionSubclass::NumericValueOutOfRange,
                "integer absolute value overflow",
                span,
            )
        }),
        Value::Uint(value) => Ok(Value::Uint(value)),
        Value::Uint128(value) => Ok(Value::Uint128(value)),
        Value::Float(value) if value.is_finite() => Ok(Value::Float(value.abs())),
        Value::Float32(value) if value.is_finite() => Ok(Value::Float(f64::from(value).abs())),
        Value::Float(_) | Value::Float32(_) => data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "numeric result is non-finite",
            span,
        ),
        _ => data_exception("abs argument is not numeric", span),
    }
}

fn eval_unary_numeric(
    _name: &str,
    args: Vec<Value>,
    span: SourceSpan,
    op: fn(f64) -> f64,
) -> Result<Value, ExecutorError> {
    match args.into_iter().next().expect("arity checked") {
        Value::Null => Ok(Value::Null),
        value @ (Value::Int(_) | Value::Uint(_) | Value::Int128(_) | Value::Uint128(_)) => {
            Ok(value)
        }
        value => {
            let Some(value) = numeric_to_f64(&value) else {
                return data_exception("numeric function argument is not numeric", span);
            };
            let value = op(value);
            if value.is_finite() {
                Ok(Value::Float(value))
            } else {
                data_exception_with(
                    DataExceptionSubclass::NumericValueOutOfRange,
                    "numeric result is non-finite",
                    span,
                )
            }
        }
    }
}

fn eval_unary_float(
    args: Vec<Value>,
    span: SourceSpan,
    op: fn(f64) -> f64,
    non_finite_message: &'static str,
) -> Result<Value, ExecutorError> {
    let Some(value) = eval_float_arg(args, span)? else {
        return Ok(Value::Null);
    };
    finite_float(op(value), non_finite_message, span)
}

fn eval_inverse_trig(
    args: Vec<Value>,
    span: SourceSpan,
    op: fn(f64) -> f64,
) -> Result<Value, ExecutorError> {
    let Some(value) = eval_float_arg(args, span)? else {
        return Ok(Value::Null);
    };
    if !(-1.0..=1.0).contains(&value) {
        return data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "inverse trigonometric argument is outside [-1, 1]",
            span,
        );
    }
    finite_float(op(value), "trigonometric result is non-finite", span)
}

fn eval_cot(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let Some(value) = eval_float_arg(args, span)? else {
        return Ok(Value::Null);
    };
    let tangent = value.tan();
    if !tangent.is_finite() || tangent == 0.0 {
        return data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "cotangent divisor is zero or non-finite",
            span,
        );
    }
    finite_float(1.0 / tangent, "cotangent result is non-finite", span)
}

fn eval_float_arg(args: Vec<Value>, span: SourceSpan) -> Result<Option<f64>, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(None);
    }
    numeric_to_f64(&value)
        .ok_or_else(|| data_exception_value("numeric function argument is not numeric", span))
        .map(Some)
}

fn finite_float(
    value: f64,
    non_finite_message: &'static str,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if value.is_finite() {
        Ok(Value::Float(value))
    } else {
        data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            non_finite_message,
            span,
        )
    }
}

fn eval_ln(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let Some(value) = eval_float_arg(args, span)? else {
        return Ok(Value::Null);
    };
    if value <= 0.0 {
        return data_exception_with(
            DataExceptionSubclass::InvalidArgumentForNaturalLogarithm,
            "natural logarithm argument is zero or negative",
            span,
        );
    }
    finite_float(value.ln(), "natural logarithm result is non-finite", span)
}

fn eval_log(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    if args.iter().any(|value| matches!(value, Value::Null)) {
        return Ok(Value::Null);
    }
    let Some(base) = numeric_to_f64(&args[0]) else {
        return data_exception("logarithm base is not numeric", span);
    };
    let Some(argument) = numeric_to_f64(&args[1]) else {
        return data_exception("logarithm argument is not numeric", span);
    };
    eval_log_values(base, argument, span)
}

fn eval_log10(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let Some(argument) = eval_float_arg(args, span)? else {
        return Ok(Value::Null);
    };
    eval_log_values(10.0, argument, span)
}

fn eval_log_values(base: f64, argument: f64, span: SourceSpan) -> Result<Value, ExecutorError> {
    if argument <= 0.0 {
        return data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "logarithm argument is zero or negative",
            span,
        );
    }
    if base <= 0.0 || base == 1.0 {
        return data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "logarithm base is zero, negative, or one",
            span,
        );
    }
    finite_float(argument.log(base), "logarithm result is non-finite", span)
}

fn eval_exp(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let Some(value) = eval_float_arg(args, span)? else {
        return Ok(Value::Null);
    };
    finite_float(value.exp(), "exponential result is non-finite", span)
}

fn eval_sqrt(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = numeric_to_f64(&value) else {
        return data_exception("sqrt argument is not numeric", span);
    };
    if value < 0.0 {
        return data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "sqrt argument is negative",
            span,
        );
    }
    let value = value.sqrt();
    if value.is_finite() {
        Ok(Value::Float(value))
    } else {
        data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "sqrt result is non-finite",
            span,
        )
    }
}

fn eval_length(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&value) else {
        return data_exception("length argument is not a string", span);
    };
    Ok(Value::Int(value.chars().count() as i64))
}

fn eval_substring(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
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

fn eval_string_transform(
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

fn eval_trim(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let value = args.into_iter().next().expect("arity checked");
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&value) else {
        return data_exception("trim argument is not a string", span);
    };
    Ok(Value::ExternalString(Arc::from(value.trim())))
}

fn eval_coalesce(
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

fn eval_nullif(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    let lhs = args[0].clone();
    let rhs = args[1].clone();
    match eval_equality(BinaryOp::Eq, &lhs, &rhs)? {
        Value::Bool(true) => Ok(Value::Null),
        Value::Bool(false) | Value::Null => Ok(lhs),
        _ => data_exception("nullif comparison did not produce boolean", span),
    }
}

fn eval_size(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
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

fn eval_fixed_args(
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

fn eval_range_args(
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

fn single_segment_name(name: &NonEmpty<IStr>) -> Option<String> {
    (name.len() == 1).then(|| name.first().as_str().to_ascii_lowercase())
}

fn display_name(name: &NonEmpty<IStr>) -> String {
    name.iter()
        .map(|segment| segment.as_str())
        .collect::<Vec<_>>()
        .join(".")
}
