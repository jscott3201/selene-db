use std::cmp::Ordering;

use rust_decimal::prelude::ToPrimitive;
use selene_core::Value;

use crate::{
    BinaryOp, SourceSpan, UnaryOp, ValueExpr,
    runtime::{
        Binding, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError, evaluator,
        value_compare,
    },
};

pub(super) fn eval_binary(
    op: BinaryOp,
    lhs: Value,
    rhs: Value,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match op {
        BinaryOp::And => eval_and(lhs, rhs, span),
        BinaryOp::Or => eval_or(lhs, rhs, span),
        BinaryOp::Eq | BinaryOp::Ne => eval_equality(op, &lhs, &rhs),
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            eval_ordering(op, lhs, rhs, span)
        }
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
            eval_arithmetic(op, lhs, rhs, span)
        }
        BinaryOp::Power => eval_power(lhs, rhs, span),
        BinaryOp::Xor => eval_xor(lhs, rhs, span),
        BinaryOp::Concat => eval_concat(lhs, rhs, span),
        BinaryOp::Contains => eval_string_predicate(lhs, rhs, span, |lhs, rhs| lhs.contains(rhs)),
        BinaryOp::StartsWith => {
            eval_string_predicate(lhs, rhs, span, |lhs, rhs| lhs.starts_with(rhs))
        }
        BinaryOp::EndsWith => eval_string_predicate(lhs, rhs, span, |lhs, rhs| lhs.ends_with(rhs)),
    }
}

pub(super) fn eval_unary(
    op: UnaryOp,
    value: Value,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match op {
        UnaryOp::Not => match value {
            Value::Bool(value) => Ok(Value::Bool(!value)),
            Value::Null => Ok(Value::Null),
            _ => data_exception("NOT operand is not boolean", span),
        },
        // Negate every numeric `Value` variant. The analyzer types `- $p` as
        // Dynamic (and `- NULL` as NULL) and passes them through, so this is the
        // sole runtime guard; an unsigned operand promotes to the signed width
        // and reports `NumericValueOutOfRange` (22003) when it cannot fit.
        UnaryOp::Negate => match value {
            Value::Int(value) => value
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| negate_overflow(span)),
            Value::Int128(value) => value
                .checked_neg()
                .map(Value::Int128)
                .ok_or_else(|| negate_overflow(span)),
            Value::Uint(value) => i64::try_from(value)
                .ok()
                .and_then(i64::checked_neg)
                .map(Value::Int)
                .ok_or_else(|| negate_overflow(span)),
            Value::Uint128(value) => i128::try_from(value)
                .ok()
                .and_then(i128::checked_neg)
                .map(Value::Int128)
                .ok_or_else(|| negate_overflow(span)),
            Value::Float(value) => Ok(Value::Float(-value)),
            Value::Float32(value) => Ok(Value::Float32(-value)),
            Value::Decimal(value) => Ok(Value::Decimal(-value)),
            Value::Null => Ok(Value::Null),
            _ => data_exception("unary minus operand is not numeric", span),
        },
    }
}

fn negate_overflow(span: SourceSpan) -> ExecutorError {
    data_exception_value_with(
        DataExceptionSubclass::NumericValueOutOfRange,
        "negation overflow: result is out of the signed integer range",
        span,
    )
}

fn eval_and(lhs: Value, rhs: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match (truth(lhs, span)?, truth(rhs, span)?) {
        (Some(false), _) | (_, Some(false)) => Ok(Value::Bool(false)),
        (Some(true), Some(true)) => Ok(Value::Bool(true)),
        _ => Ok(Value::Null),
    }
}

fn eval_or(lhs: Value, rhs: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match (truth(lhs, span)?, truth(rhs, span)?) {
        (Some(true), _) | (_, Some(true)) => Ok(Value::Bool(true)),
        (Some(false), Some(false)) => Ok(Value::Bool(false)),
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

fn eval_xor(lhs: Value, rhs: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match (truth(lhs, span)?, truth(rhs, span)?) {
        (Some(lhs), Some(rhs)) => Ok(Value::Bool(lhs ^ rhs)),
        _ => Ok(Value::Null),
    }
}

pub(super) fn eval_equality(
    op: BinaryOp,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, ExecutorError> {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(equal) = value_compare::gql_equal_non_null(lhs, rhs) else {
        return Ok(Value::Null);
    };
    Ok(Value::Bool(match op {
        BinaryOp::Eq => equal,
        BinaryOp::Ne => !equal,
        _ => unreachable!("guarded by caller"),
    }))
}

pub(super) fn eval_ordering(
    op: BinaryOp,
    lhs: Value,
    rhs: Value,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(ordering) = value_compare::compare_non_null(&lhs, &rhs) else {
        return data_exception_with(
            DataExceptionSubclass::ValuesNotComparable,
            "values are not order-comparable",
            span,
        );
    };
    Ok(Value::Bool(match op {
        BinaryOp::Lt => ordering == Ordering::Less,
        BinaryOp::Le => matches!(ordering, Ordering::Less | Ordering::Equal),
        BinaryOp::Gt => ordering == Ordering::Greater,
        BinaryOp::Ge => matches!(ordering, Ordering::Greater | Ordering::Equal),
        _ => unreachable!("guarded by caller"),
    }))
}

fn eval_arithmetic(
    op: BinaryOp,
    lhs: Value,
    rhs: Value,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return Ok(Value::Null);
    }
    match (lhs, rhs) {
        (Value::Int(lhs), Value::Int(rhs)) => eval_int_arithmetic(op, lhs, rhs, span),
        (Value::Uint(lhs), Value::Uint(rhs)) => eval_uint_arithmetic(op, lhs, rhs, span),
        (Value::Int128(lhs), Value::Int128(rhs)) => eval_i128_arithmetic(op, lhs, rhs, span),
        (Value::Int128(lhs), Value::Int(rhs)) => {
            eval_i128_arithmetic(op, lhs, i128::from(rhs), span)
        }
        (Value::Int(lhs), Value::Int128(rhs)) => {
            eval_i128_arithmetic(op, i128::from(lhs), rhs, span)
        }
        (Value::Int128(lhs), Value::Uint(rhs)) => {
            eval_i128_arithmetic(op, lhs, i128::from(rhs), span)
        }
        (Value::Uint(lhs), Value::Int128(rhs)) => {
            eval_i128_arithmetic(op, i128::from(lhs), rhs, span)
        }
        (Value::Int128(lhs), Value::Float(rhs)) => eval_float_arithmetic(op, lhs as f64, rhs, span),
        (Value::Float(lhs), Value::Int128(rhs)) => eval_float_arithmetic(op, lhs, rhs as f64, span),
        (Value::Int128(lhs), Value::Float32(rhs)) => {
            eval_float_arithmetic(op, lhs as f64, f64::from(rhs), span)
        }
        (Value::Float32(lhs), Value::Int128(rhs)) => {
            eval_float_arithmetic(op, f64::from(lhs), rhs as f64, span)
        }
        (Value::Uint128(lhs), Value::Float(rhs)) => {
            eval_float_arithmetic(op, lhs as f64, rhs, span)
        }
        (Value::Float(lhs), Value::Uint128(rhs)) => {
            eval_float_arithmetic(op, lhs, rhs as f64, span)
        }
        (Value::Uint128(lhs), Value::Float32(rhs)) => {
            eval_float_arithmetic(op, lhs as f64, f64::from(rhs), span)
        }
        (Value::Float32(lhs), Value::Uint128(rhs)) => {
            eval_float_arithmetic(op, f64::from(lhs), rhs as f64, span)
        }
        (Value::Int(lhs), Value::Uint(rhs)) => {
            eval_i128_arithmetic(op, i128::from(lhs), i128::from(rhs), span)
        }
        (Value::Uint(lhs), Value::Int(rhs)) => {
            eval_i128_arithmetic(op, i128::from(lhs), i128::from(rhs), span)
        }
        // 128-bit unsigned arithmetic (GQLRT-30). `Uint128 + Uint128` stays
        // unsigned (overflow → 22003); a `Uint`/`Uint128` mix widens to u128.
        (Value::Uint128(lhs), Value::Uint128(rhs)) => eval_u128_arithmetic(op, lhs, rhs, span),
        (Value::Uint128(lhs), Value::Uint(rhs)) => {
            eval_u128_arithmetic(op, lhs, u128::from(rhs), span)
        }
        (Value::Uint(lhs), Value::Uint128(rhs)) => {
            eval_u128_arithmetic(op, u128::from(lhs), rhs, span)
        }
        // 128-bit signed/unsigned mix folds into i128 (the unsigned side must
        // fit i128; out-of-range → 22003).
        (Value::Int128(lhs), Value::Uint128(rhs)) => {
            eval_i128_arithmetic(op, lhs, i128_from_u128(rhs, span)?, span)
        }
        (Value::Uint128(lhs), Value::Int128(rhs)) => {
            eval_i128_arithmetic(op, i128_from_u128(lhs, span)?, rhs, span)
        }
        // DECIMAL arithmetic (GQLRT-30). Decimal op Decimal/integer stays in the
        // exact base-10 channel; Decimal op float collapses to f64 below.
        (Value::Decimal(lhs), Value::Decimal(rhs)) => eval_decimal_arithmetic(op, lhs, rhs, span),
        (Value::Decimal(lhs), Value::Int(rhs)) => {
            eval_decimal_arithmetic(op, lhs, rust_decimal::Decimal::from(rhs), span)
        }
        (Value::Int(lhs), Value::Decimal(rhs)) => {
            eval_decimal_arithmetic(op, rust_decimal::Decimal::from(lhs), rhs, span)
        }
        (Value::Decimal(lhs), Value::Uint(rhs)) => {
            eval_decimal_arithmetic(op, lhs, rust_decimal::Decimal::from(rhs), span)
        }
        (Value::Uint(lhs), Value::Decimal(rhs)) => {
            eval_decimal_arithmetic(op, rust_decimal::Decimal::from(lhs), rhs, span)
        }
        (Value::Decimal(lhs), Value::Int128(rhs)) => {
            eval_decimal_arithmetic(op, lhs, decimal_from_i128(rhs, span)?, span)
        }
        (Value::Int128(lhs), Value::Decimal(rhs)) => {
            eval_decimal_arithmetic(op, decimal_from_i128(lhs, span)?, rhs, span)
        }
        (Value::Decimal(lhs), Value::Uint128(rhs)) => eval_decimal_arithmetic(
            op,
            lhs,
            decimal_from_i128(i128_from_u128(rhs, span)?, span)?,
            span,
        ),
        (Value::Uint128(lhs), Value::Decimal(rhs)) => eval_decimal_arithmetic(
            op,
            decimal_from_i128(i128_from_u128(lhs, span)?, span)?,
            rhs,
            span,
        ),
        // Any remaining numeric mix involving a binary float (e.g. Decimal +
        // Float, Int128 + Float already handled above) collapses to f64.
        (lhs, rhs) => {
            let (Some(lhs), Some(rhs)) = (numeric_to_f64(&lhs), numeric_to_f64(&rhs)) else {
                return data_exception("arithmetic operands are not numeric", span);
            };
            eval_float_arithmetic(op, lhs, rhs, span)
        }
    }
}

fn eval_power(lhs: Value, rhs: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return Ok(Value::Null);
    }
    if let (Value::Int(lhs), Value::Int(rhs)) = (&lhs, &rhs)
        && *rhs >= 0
    {
        let exponent = u32::try_from(*rhs).map_err(|_| {
            data_exception_value_with(
                DataExceptionSubclass::NumericValueOutOfRange,
                "integer exponent is negative or too large",
                span,
            )
        })?;
        return lhs.checked_pow(exponent).map(Value::Int).ok_or_else(|| {
            data_exception_value_with(
                DataExceptionSubclass::NumericValueOutOfRange,
                "integer exponentiation overflow",
                span,
            )
        });
    }
    let (Some(lhs), Some(rhs)) = (numeric_to_f64(&lhs), numeric_to_f64(&rhs)) else {
        return data_exception("power operands are not numeric", span);
    };
    eval_float_power(lhs, rhs, span)
}

fn eval_float_power(lhs: f64, rhs: f64, span: SourceSpan) -> Result<Value, ExecutorError> {
    if lhs.is_nan() || rhs.is_nan() {
        return finite_power_result(f64::NAN, span);
    }
    if lhs == 0.0 {
        if rhs < 0.0 {
            return invalid_power_argument("power base is zero and exponent is negative", span);
        }
        if rhs == 0.0 {
            return Ok(Value::Float(1.0));
        }
        return Ok(Value::Float(0.0));
    }
    if lhs < 0.0 {
        if !is_integral(rhs) {
            return invalid_power_argument(
                "power base is negative and exponent is not an integer",
                span,
            );
        }
        let magnitude = (-lhs).powf(rhs);
        let value = if is_even_integer(rhs) {
            magnitude
        } else {
            -magnitude
        };
        return finite_power_result(value, span);
    }
    finite_power_result(lhs.powf(rhs), span)
}

fn invalid_power_argument(message: &'static str, span: SourceSpan) -> Result<Value, ExecutorError> {
    data_exception_with(
        DataExceptionSubclass::InvalidArgumentForPowerFunction,
        message,
        span,
    )
}

fn finite_power_result(value: f64, span: SourceSpan) -> Result<Value, ExecutorError> {
    if value.is_finite() {
        Ok(Value::Float(value))
    } else {
        data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "floating-point exponentiation produced non-finite value",
            span,
        )
    }
}

fn is_integral(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0
}

fn is_even_integer(value: f64) -> bool {
    value.rem_euclid(2.0) == 0.0
}

fn eval_concat(lhs: Value, rhs: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return Ok(Value::Null);
    }
    match (lhs, rhs) {
        (Value::String(lhs), Value::String(rhs)) => intern_string(&format!("{lhs}{rhs}"), span),
        (Value::List(mut lhs), Value::List(rhs)) => {
            lhs.extend(rhs);
            Ok(Value::List(lhs))
        }
        _ => data_exception(
            "concatenation operands must both be strings or both be lists",
            span,
        ),
    }
}

fn eval_string_predicate(
    lhs: Value,
    rhs: Value,
    span: SourceSpan,
    predicate: impl Fn(&str, &str) -> bool,
) -> Result<Value, ExecutorError> {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return Ok(Value::Null);
    }
    let (Some(lhs), Some(rhs)) = (string_slice(&lhs), string_slice(&rhs)) else {
        return data_exception("string predicate operands are not both strings", span);
    };
    Ok(Value::Bool(predicate(lhs, rhs)))
}

fn eval_int_arithmetic(
    op: BinaryOp,
    lhs: i64,
    rhs: i64,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let value = match op {
        BinaryOp::Add => lhs.checked_add(rhs),
        BinaryOp::Sub => lhs.checked_sub(rhs),
        BinaryOp::Mul => lhs.checked_mul(rhs),
        BinaryOp::Div => (rhs != 0).then(|| lhs.checked_div(rhs)).flatten(),
        BinaryOp::Mod => (rhs != 0).then(|| lhs.checked_rem(rhs)).flatten(),
        _ => None,
    };
    value.map(Value::Int).ok_or_else(|| {
        let subclass = if matches!(op, BinaryOp::Div | BinaryOp::Mod) && rhs == 0 {
            DataExceptionSubclass::DivisionByZero
        } else {
            DataExceptionSubclass::NumericValueOutOfRange
        };
        data_exception_value_with(
            subclass,
            "integer arithmetic overflow or division by zero",
            span,
        )
    })
}

fn eval_uint_arithmetic(
    op: BinaryOp,
    lhs: u64,
    rhs: u64,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let value = match op {
        BinaryOp::Add => lhs.checked_add(rhs),
        BinaryOp::Sub => lhs.checked_sub(rhs),
        BinaryOp::Mul => lhs.checked_mul(rhs),
        BinaryOp::Div => (rhs != 0).then(|| lhs.checked_div(rhs)).flatten(),
        BinaryOp::Mod => (rhs != 0).then(|| lhs.checked_rem(rhs)).flatten(),
        _ => None,
    };
    if let Some(value) = value {
        return Ok(Value::Uint(value));
    }
    eval_i128_arithmetic(op, i128::from(lhs), i128::from(rhs), span)
}

fn eval_i128_arithmetic(
    op: BinaryOp,
    lhs: i128,
    rhs: i128,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let value = match op {
        BinaryOp::Add => lhs.checked_add(rhs),
        BinaryOp::Sub => lhs.checked_sub(rhs),
        BinaryOp::Mul => lhs.checked_mul(rhs),
        BinaryOp::Div => (rhs != 0).then(|| lhs.checked_div(rhs)).flatten(),
        BinaryOp::Mod => (rhs != 0).then(|| lhs.checked_rem(rhs)).flatten(),
        _ => None,
    };
    value.map(Value::Int128).ok_or_else(|| {
        let subclass = if matches!(op, BinaryOp::Div | BinaryOp::Mod) && rhs == 0 {
            DataExceptionSubclass::DivisionByZero
        } else {
            DataExceptionSubclass::NumericValueOutOfRange
        };
        data_exception_value_with(
            subclass,
            "integer arithmetic overflow or division by zero",
            span,
        )
    })
}

fn eval_u128_arithmetic(
    op: BinaryOp,
    lhs: u128,
    rhs: u128,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let value = match op {
        BinaryOp::Add => lhs.checked_add(rhs),
        BinaryOp::Sub => lhs.checked_sub(rhs),
        BinaryOp::Mul => lhs.checked_mul(rhs),
        BinaryOp::Div => (rhs != 0).then(|| lhs.checked_div(rhs)).flatten(),
        BinaryOp::Mod => (rhs != 0).then(|| lhs.checked_rem(rhs)).flatten(),
        _ => None,
    };
    value.map(Value::Uint128).ok_or_else(|| {
        let subclass = if matches!(op, BinaryOp::Div | BinaryOp::Mod) && rhs == 0 {
            DataExceptionSubclass::DivisionByZero
        } else {
            // No wider unsigned type exists, so overflow is a hard 22003.
            DataExceptionSubclass::NumericValueOutOfRange
        };
        data_exception_value_with(
            subclass,
            "unsigned 128-bit arithmetic overflow or division by zero",
            span,
        )
    })
}

fn eval_decimal_arithmetic(
    op: BinaryOp,
    lhs: rust_decimal::Decimal,
    rhs: rust_decimal::Decimal,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let value = match op {
        BinaryOp::Add => lhs.checked_add(rhs),
        BinaryOp::Sub => lhs.checked_sub(rhs),
        BinaryOp::Mul => lhs.checked_mul(rhs),
        BinaryOp::Div => (!rhs.is_zero()).then(|| lhs.checked_div(rhs)).flatten(),
        BinaryOp::Mod => (!rhs.is_zero()).then(|| lhs.checked_rem(rhs)).flatten(),
        _ => None,
    };
    value.map(Value::Decimal).ok_or_else(|| {
        let subclass = if matches!(op, BinaryOp::Div | BinaryOp::Mod) && rhs.is_zero() {
            DataExceptionSubclass::DivisionByZero
        } else {
            DataExceptionSubclass::NumericValueOutOfRange
        };
        data_exception_value_with(
            subclass,
            "decimal arithmetic overflow or division by zero",
            span,
        )
    })
}

/// Narrow a `u128` to `i128` for signed/unsigned 128-bit arithmetic, raising
/// 22003 when it exceeds the signed range.
fn i128_from_u128(value: u128, span: SourceSpan) -> Result<i128, ExecutorError> {
    i128::try_from(value).map_err(|_| {
        data_exception_value_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "unsigned 128-bit value exceeds the signed integer range",
            span,
        )
    })
}

/// Promote an `i128` to `Decimal` for mixed Decimal arithmetic, raising 22003
/// when it exceeds Decimal's range.
fn decimal_from_i128(
    value: i128,
    span: SourceSpan,
) -> Result<rust_decimal::Decimal, ExecutorError> {
    rust_decimal::Decimal::try_from_i128_with_scale(value, 0).map_err(|_| {
        data_exception_value_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "128-bit value exceeds the DECIMAL range",
            span,
        )
    })
}

fn eval_float_arithmetic(
    op: BinaryOp,
    lhs: f64,
    rhs: f64,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let value = match op {
        BinaryOp::Add => lhs + rhs,
        BinaryOp::Sub => lhs - rhs,
        BinaryOp::Mul => lhs * rhs,
        BinaryOp::Div if rhs != 0.0 => lhs / rhs,
        BinaryOp::Mod if rhs != 0.0 => lhs % rhs,
        _ => {
            return data_exception_with(
                DataExceptionSubclass::DivisionByZero,
                "floating-point division by zero",
                span,
            );
        }
    };
    if value.is_finite() {
        Ok(Value::Float(value))
    } else {
        data_exception_with(
            DataExceptionSubclass::NumericValueOutOfRange,
            "floating-point arithmetic produced non-finite value",
            span,
        )
    }
}

pub(super) fn eval_in_list(
    value: Value,
    list: &[ValueExpr],
    negated: bool,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let mut saw_unknown = false;
    for item in list {
        let item = evaluator::evaluate(item, binding, schema, ctx)?;
        if matches!(item, Value::Null) {
            saw_unknown = true;
            continue;
        }
        let comparison = eval_equality(BinaryOp::Eq, &value, &item)?;
        match comparison {
            Value::Bool(true) => return Ok(Value::Bool(!negated)),
            Value::Bool(false) => {}
            Value::Null => saw_unknown = true,
            _ => return data_exception("IN comparison did not produce boolean", span),
        }
    }
    if saw_unknown {
        Ok(Value::Null)
    } else {
        Ok(Value::Bool(negated))
    }
}

pub(super) fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(value) => Some(*value as f64),
        Value::Uint(value) => Some(*value as f64),
        Value::Float(value) => Some(*value),
        Value::Float32(value) => Some(f64::from(*value)),
        _ => None,
    }
}

pub(super) fn numeric_to_f64(value: &Value) -> Option<f64> {
    as_f64(value).or(match value {
        Value::Int128(value) => Some(*value as f64),
        Value::Uint128(value) => Some(*value as f64),
        Value::Decimal(value) => value.to_f64(),
        _ => None,
    })
}

pub(super) fn string_slice(value: &Value) -> Option<&str> {
    match value {
        Value::String(value) => Some(value.as_str()),
        _ => None,
    }
}

/// Construct a `Value::String` from engine-produced text, mapping the IL013
/// byte-cap failure to a runtime data exception at `span`.
pub(super) fn intern_string(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    match selene_core::intern(text) {
        Ok(istr) => Ok(Value::String(istr)),
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
