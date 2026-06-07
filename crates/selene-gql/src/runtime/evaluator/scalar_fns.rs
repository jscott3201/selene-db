//! Scalar function evaluation.
//!
//! Dispatches the v1.1 closed scalar-function set case-insensitively. Each
//! function owns arity checking; `NULL` propagates except where short-circuit
//! functions (`coalesce`, `nullif`) define different behavior.

use selene_core::{DbString, Value};

use crate::{
    BinaryOp, NonEmpty, SourceSpan, ValueExpr,
    runtime::{Binding, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError},
};

use super::{
    binary_ops::{
        data_exception, data_exception_value, data_exception_value_with, data_exception_with,
        eval_binary, numeric_to_f64,
    },
    identity_length_fns, json_fns,
    string_fns::{self, eval_fixed_args, eval_range_args},
    temporal_fns, uuid_fns,
};

pub(super) fn eval_function_call(
    name: &NonEmpty<DbString>,
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
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            f64::ceil,
        ),
        "floor" => eval_unary_numeric(
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
        "length" | "char_length" | "character_length" => string_fns::eval_length(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "left" => string_fns::eval_left_right(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
            false,
        ),
        "right" => string_fns::eval_left_right(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
            true,
        ),
        "btrim" => string_fns::eval_multi_char_trim(
            eval_range_args(&display_name, args, 1..=2, span, binding, schema, ctx)?,
            span,
            string_fns::TrimSide::Both,
        ),
        "ltrim" => string_fns::eval_multi_char_trim(
            eval_range_args(&display_name, args, 1..=2, span, binding, schema, ctx)?,
            span,
            string_fns::TrimSide::Leading,
        ),
        "rtrim" => string_fns::eval_multi_char_trim(
            eval_range_args(&display_name, args, 1..=2, span, binding, schema, ctx)?,
            span,
            string_fns::TrimSide::Trailing,
        ),
        "upper" => string_fns::eval_string_transform(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            str::to_uppercase,
        ),
        "lower" => string_fns::eval_string_transform(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
            str::to_lowercase,
        ),
        "trim" => string_fns::eval_trim(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "coalesce" => string_fns::eval_coalesce(&display_name, args, span, binding, schema, ctx),
        "nullif" => string_fns::eval_nullif(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
        ),
        "size" => string_fns::eval_size(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "uuid_v4" => uuid_fns::eval_uuid_v4(eval_fixed_args(
            &display_name,
            args,
            0,
            span,
            binding,
            schema,
            ctx,
        )?),
        "uuid_v7" => uuid_fns::eval_uuid_v7(eval_fixed_args(
            &display_name,
            args,
            0,
            span,
            binding,
            schema,
            ctx,
        )?),
        "uuid" => uuid_fns::eval_uuid(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "json" | "json_parse" => json_fns::eval_json_parse(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "json_stringify" => json_fns::eval_json_stringify(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "json_type" => json_fns::eval_json_type(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "json_array" => json_fns::eval_json_array(
            eval_range_args(
                &display_name,
                args,
                0..=json_fns::JSON_ARRAY_MAX_ARGS,
                span,
                binding,
                schema,
                ctx,
            )?,
            span,
        ),
        "json_object" => json_fns::eval_json_object(
            eval_range_args(
                &display_name,
                args,
                0..=json_fns::JSON_OBJECT_MAX_ARGS,
                span,
                binding,
                schema,
                ctx,
            )?,
            span,
        ),
        "json_array_length" => json_fns::eval_json_array_length(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "json_object_keys" => json_fns::eval_json_object_keys(
            eval_fixed_args(&display_name, args, 1, span, binding, schema, ctx)?,
            span,
        ),
        "json_contains" => json_fns::eval_json_contains(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
        ),
        "json_merge_patch" => json_fns::eval_json_merge_patch(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
        ),
        "json_patch" => json_fns::eval_json_patch(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
        ),
        "json_get" => json_fns::eval_json_get(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
        ),
        "json_get_text" => json_fns::eval_json_get_text(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
        ),
        "json_get_scalar" => json_fns::eval_json_get_scalar(
            eval_fixed_args(&display_name, args, 2, span, binding, schema, ctx)?,
            span,
        ),
        "json_get_path" => json_fns::eval_json_get_path(
            eval_range_args(
                &display_name,
                args,
                2..=json_fns::JSON_PATH_MAX_ARGS,
                span,
                binding,
                schema,
                ctx,
            )?,
            span,
        ),
        "json_get_path_text" => json_fns::eval_json_get_path_text(
            eval_range_args(
                &display_name,
                args,
                2..=json_fns::JSON_PATH_MAX_ARGS,
                span,
                binding,
                schema,
                ctx,
            )?,
            span,
        ),
        "json_get_path_scalar" => json_fns::eval_json_get_path_scalar(
            eval_range_args(
                &display_name,
                args,
                2..=json_fns::JSON_PATH_MAX_ARGS,
                span,
                binding,
                schema,
                ctx,
            )?,
            span,
        ),
        "json_has_path" => json_fns::eval_json_has_path(
            eval_range_args(
                &display_name,
                args,
                2..=json_fns::JSON_PATH_MAX_ARGS,
                span,
                binding,
                schema,
                ctx,
            )?,
            span,
        ),
        // ISO/IEC 39075:2024 section 20.27 current-datetime functions. Each is
        // niladic and reads the session time zone threaded into the context.
        "current_timestamp" | "now" => {
            eval_fixed_args(&display_name, args, 0, span, binding, schema, ctx)?;
            temporal_fns::eval_current_timestamp(ctx)
        }
        "localtimestamp" => {
            eval_fixed_args(&display_name, args, 0, span, binding, schema, ctx)?;
            temporal_fns::eval_localtimestamp(ctx)
        }
        "current_date" => {
            eval_fixed_args(&display_name, args, 0, span, binding, schema, ctx)?;
            temporal_fns::eval_current_date(ctx)
        }
        "current_time" => {
            eval_fixed_args(&display_name, args, 0, span, binding, schema, ctx)?;
            temporal_fns::eval_current_time(ctx)
        }
        "localtime" => {
            eval_fixed_args(&display_name, args, 0, span, binding, schema, ctx)?;
            temporal_fns::eval_localtime(ctx)
        }
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

fn single_segment_name(name: &NonEmpty<DbString>) -> Option<String> {
    (name.len() == 1).then(|| name.first().as_str().to_ascii_lowercase())
}

fn display_name(name: &NonEmpty<DbString>) -> String {
    name.iter()
        .map(|segment| segment.as_str())
        .collect::<Vec<_>>()
        .join(".")
}
