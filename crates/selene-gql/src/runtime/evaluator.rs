//! Residual-filter expression evaluator.

use std::cmp::Ordering;

use selene_core::{EdgeId, NodeId, Value};

use crate::{
    BinaryOp, IsCheckKind, Literal, SourceSpan, UnaryOp, ValueExpr,
    runtime::{Binding, BindingTableSchema, ExecutorError, TxContext, value_compare},
};

/// Evaluate a value expression against one binding-table row.
pub fn evaluate(
    expr: &ValueExpr,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &TxContext<'_>,
) -> Result<Value, ExecutorError> {
    match expr {
        ValueExpr::Literal(literal) => Ok(literal_value(literal)),
        ValueExpr::Variable { name, span } => lookup_variable(*name, *span, binding, schema),
        ValueExpr::PropertyAccess { target, key, .. } => {
            let target = evaluate(target, binding, schema, ctx)?;
            property_access(&target, *key, ctx)
        }
        ValueExpr::BinaryOp { op, lhs, rhs, span } => {
            let lhs = evaluate(lhs, binding, schema, ctx)?;
            let rhs = evaluate(rhs, binding, schema, ctx)?;
            eval_binary(*op, lhs, rhs, *span)
        }
        ValueExpr::UnaryOp { op, operand, span } => {
            let value = evaluate(operand, binding, schema, ctx)?;
            eval_unary(*op, value, *span)
        }
        ValueExpr::IsCheck {
            operand,
            kind: IsCheckKind::Null,
            negated,
            ..
        } => {
            let is_null = matches!(evaluate(operand, binding, schema, ctx)?, Value::Null);
            Ok(Value::Bool(if *negated { !is_null } else { is_null }))
        }
        ValueExpr::InList {
            operand,
            list,
            negated,
            span,
        } => {
            let value = evaluate(operand, binding, schema, ctx)?;
            eval_in_list(value, list, *negated, *span, binding, schema, ctx)
        }
        ValueExpr::FunctionCall { .. } => Err(ExecutorError::ImplementationDefined {
            detail: "function call evaluation not implemented",
        }),
        ValueExpr::Case { .. } => Err(ExecutorError::ImplementationDefined {
            detail: "CASE evaluation not implemented",
        }),
        ValueExpr::Exists { .. } => Err(ExecutorError::ImplementationDefined {
            detail: "EXISTS subquery evaluation not implemented",
        }),
        ValueExpr::CountSubquery { .. } => Err(ExecutorError::ImplementationDefined {
            detail: "COUNT subquery evaluation not implemented",
        }),
        ValueExpr::Parameter { .. }
        | ValueExpr::ListAccess { .. }
        | ValueExpr::ListLiteral { .. }
        | ValueExpr::RecordLiteral { .. }
        | ValueExpr::IsCheck { .. }
        | ValueExpr::Like { .. }
        | ValueExpr::Between { .. }
        | ValueExpr::AllDifferent { .. }
        | ValueExpr::Same { .. }
        | ValueExpr::PropertyExists { .. } => Err(ExecutorError::ImplementationDefined {
            detail: "expression kind not implemented",
        }),
    }
}

fn lookup_variable(
    name: selene_core::IStr,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
) -> Result<Value, ExecutorError> {
    let Some(index) = schema
        .columns
        .iter()
        .position(|column| column.name == Some(name))
    else {
        return Ok(Value::Null);
    };
    binding
        .get(index)
        .cloned()
        .ok_or_else(|| ExecutorError::InvalidReference {
            name: name.as_str().to_owned(),
            span,
        })
}

fn property_access(
    target: &Value,
    key: selene_core::IStr,
    ctx: &TxContext<'_>,
) -> Result<Value, ExecutorError> {
    match target {
        Value::Null => Ok(Value::Null),
        Value::NodeRef(id) => Ok(property_from_node(*id, key, ctx)),
        Value::EdgeRef(id) => Ok(property_from_edge(*id, key, ctx)),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "property access target is not graph element",
        }),
    }
}

fn property_from_node(id: NodeId, key: selene_core::IStr, ctx: &TxContext<'_>) -> Value {
    ctx.snapshot()
        .node_properties(id)
        .and_then(|properties| properties.get(&key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn property_from_edge(id: EdgeId, key: selene_core::IStr, ctx: &TxContext<'_>) -> Value {
    ctx.snapshot()
        .edge_properties(id)
        .and_then(|properties| properties.get(&key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn literal_value(literal: &Literal) -> Value {
    match literal {
        Literal::Bool(value, _) => Value::Bool(*value),
        Literal::Integer(value, _) => Value::Int(*value),
        Literal::Float(value, _) => Value::Float(*value),
        Literal::String(value, _) => Value::String(*value),
        Literal::Null(_) => Value::Null,
    }
}

fn eval_binary(
    op: BinaryOp,
    lhs: Value,
    rhs: Value,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match op {
        BinaryOp::And => eval_and(lhs, rhs, span),
        BinaryOp::Or => eval_or(lhs, rhs, span),
        BinaryOp::Eq | BinaryOp::Ne => eval_equality(op, lhs, rhs),
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

fn eval_unary(op: UnaryOp, value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
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

fn eval_equality(op: BinaryOp, lhs: Value, rhs: Value) -> Result<Value, ExecutorError> {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return Ok(Value::Null);
    }
    let equal = value_compare::equal_non_null(&lhs, &rhs);
    Ok(Value::Bool(match op {
        BinaryOp::Eq => equal,
        BinaryOp::Ne => !equal,
        _ => unreachable!("guarded by caller"),
    }))
}

fn eval_ordering(
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

fn eval_in_list(
    value: Value,
    list: &[ValueExpr],
    negated: bool,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &TxContext<'_>,
) -> Result<Value, ExecutorError> {
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let mut saw_unknown = false;
    for item in list {
        let item = evaluate(item, binding, schema, ctx)?;
        if matches!(item, Value::Null) {
            saw_unknown = true;
            continue;
        }
        let comparison = eval_equality(BinaryOp::Eq, value.clone(), item)?;
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

fn data_exception<T>(message: impl Into<String>, span: SourceSpan) -> Result<T, ExecutorError> {
    Err(data_exception_value(message, span))
}

fn data_exception_value(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    ExecutorError::DataException {
        message: message.into(),
        span,
    }
}
