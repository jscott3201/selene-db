use std::cmp::Ordering;

use selene_core::Value;

use crate::{
    BinaryOp, SourceSpan, UnaryOp, ValueExpr,
    runtime::{Binding, BindingTableSchema, ExecutorError, TxContext, evaluator, value_compare},
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
        BinaryOp::Power
        | BinaryOp::Xor
        | BinaryOp::Concat
        | BinaryOp::Contains
        | BinaryOp::StartsWith
        | BinaryOp::EndsWith => Err(ExecutorError::ImplementationDefined {
            detail: "binary operator not implemented",
        }),
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
        UnaryOp::Negate => match value {
            Value::Int(value) => value
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| data_exception_value("integer arithmetic overflow", span)),
            Value::Float(value) => Ok(Value::Float(-value)),
            Value::Null => Ok(Value::Null),
            _ => data_exception("unary minus operand is not numeric", span),
        },
    }
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
        return data_exception("values are not order-comparable", span);
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
        (lhs, rhs) => {
            let (Some(lhs), Some(rhs)) = (as_f64(&lhs), as_f64(&rhs)) else {
                return data_exception("arithmetic operands are not numeric", span);
            };
            eval_float_arithmetic(op, lhs, rhs, span)
        }
    }
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
        data_exception_value("integer arithmetic overflow or division by zero", span)
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
        data_exception_value("integer arithmetic overflow or division by zero", span)
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
        _ => return data_exception("floating-point division by zero", span),
    };
    if value.is_finite() {
        Ok(Value::Float(value))
    } else {
        data_exception("floating-point arithmetic produced non-finite value", span)
    }
}

pub(super) fn eval_in_list(
    value: Value,
    list: &[ValueExpr],
    negated: bool,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &TxContext<'_, '_>,
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

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(value) => Some(*value as f64),
        Value::Uint(value) => Some(*value as f64),
        Value::Float(value) => Some(*value),
        Value::Float32(value) => Some(f64::from(*value)),
        _ => None,
    }
}

pub(super) fn data_exception<T>(
    message: impl Into<String>,
    span: SourceSpan,
) -> Result<T, ExecutorError> {
    Err(data_exception_value(message, span))
}

pub(super) fn data_exception_value(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    ExecutorError::DataException {
        message: message.into(),
        span,
    }
}
