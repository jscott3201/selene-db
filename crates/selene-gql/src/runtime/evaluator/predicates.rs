//! Predicate expression evaluation.
//!
//! Implements residual predicate forms that are not lowered into scan access:
//! `IS` sub-kinds, graph predicate functions, `ALL_DIFFERENT`, `SAME`, and
//! `PROPERTY_EXISTS`. Predicate negation preserves `NULL` as unknown.

use selene_core::{IStr, Value};

use crate::{
    BinaryOp, GqlType, IsCheckKind, LabelExpr, SourceSpan, TruthValue, ValueExpr,
    runtime::{Binding, BindingTableSchema, EvalCtx, ExecutorError},
};

use super::{
    binary_ops::{data_exception, eval_equality, string_slice},
    evaluate, property_access, string_fns,
};
use crate::runtime::scan;

pub(super) fn eval_is_check(
    operand: &ValueExpr,
    kind: &IsCheckKind,
    negated: bool,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let operand = evaluate(operand, binding, schema, ctx)?;
    let result = match kind {
        IsCheckKind::Null => Value::Bool(matches!(operand, Value::Null)),
        IsCheckKind::Directed => eval_is_directed(operand, span, ctx)?,
        IsCheckKind::Labeled(label_expr) => eval_is_labeled(operand, label_expr, span, ctx)?,
        IsCheckKind::TruthValue(truth_value) => eval_is_truth_value(operand, *truth_value),
        IsCheckKind::Typed(ty) => Value::Bool(value_matches_type(&operand, ty)),
        IsCheckKind::SourceOf(value) => {
            eval_is_endpoint(operand, value, true, span, binding, schema, ctx)?
        }
        IsCheckKind::DestinationOf(value) => {
            eval_is_endpoint(operand, value, false, span, binding, schema, ctx)?
        }
        IsCheckKind::Normalized(form) => eval_is_normalized(operand, *form, span)?,
    };
    negate_predicate(result, negated)
}

fn eval_is_normalized(
    operand: Value,
    form: crate::NormalForm,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if matches!(operand, Value::Null) {
        return Ok(Value::Null);
    }
    let Some(value) = string_slice(&operand) else {
        return data_exception("IS NORMALIZED operand is not a string", span);
    };
    Ok(Value::Bool(string_fns::is_normalized(value, form)))
}

pub(super) fn eval_all_different(
    items: &[ValueExpr],
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let values = evaluate_items(items, binding, schema, ctx)?;
    let mut saw_unknown = false;
    for (left_index, lhs) in values.iter().enumerate() {
        if matches!(lhs, Value::Null) {
            saw_unknown = true;
            continue;
        }
        for rhs in &values[left_index + 1..] {
            if matches!(rhs, Value::Null) {
                saw_unknown = true;
                continue;
            }
            match eval_equality(BinaryOp::Eq, lhs, rhs)? {
                Value::Bool(true) => return Ok(Value::Bool(false)),
                Value::Bool(false) => {}
                Value::Null => saw_unknown = true,
                _ => {
                    return data_exception(
                        "ALL_DIFFERENT comparison did not produce boolean",
                        span,
                    );
                }
            }
        }
    }
    if saw_unknown {
        Ok(Value::Null)
    } else {
        Ok(Value::Bool(true))
    }
}

pub(super) fn eval_same(
    items: &[ValueExpr],
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let values = evaluate_items(items, binding, schema, ctx)?;
    let Some(first) = values.first() else {
        return Ok(Value::Bool(true));
    };
    if matches!(first, Value::Null) {
        return Ok(Value::Null);
    }
    match first {
        Value::NodeRef(first_id) => {
            for value in &values[1..] {
                match value {
                    Value::NodeRef(id) if id == first_id => {}
                    Value::NodeRef(_) => return Ok(Value::Bool(false)),
                    Value::Null => return Ok(Value::Null),
                    _ => return data_exception("SAME operands must all be node references", span),
                }
            }
        }
        Value::EdgeRef(first_id) => {
            for value in &values[1..] {
                match value {
                    Value::EdgeRef(id) if id == first_id => {}
                    Value::EdgeRef(_) => return Ok(Value::Bool(false)),
                    Value::Null => return Ok(Value::Null),
                    _ => return data_exception("SAME operands must all be edge references", span),
                }
            }
        }
        _ => return data_exception("SAME operands must be graph element references", span),
    }
    Ok(Value::Bool(true))
}

pub(super) fn eval_property_exists(
    target: &ValueExpr,
    key: IStr,
    _span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let target = evaluate(target, binding, schema, ctx)?;
    if matches!(target, Value::Null) {
        return Ok(Value::Null);
    }
    let value = property_access(&target, key, ctx)?;
    Ok(Value::Bool(!matches!(value, Value::Null)))
}

fn evaluate_items(
    items: &[ValueExpr],
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<Value>, ExecutorError> {
    items
        .iter()
        .map(|item| evaluate(item, binding, schema, ctx))
        .collect()
}

fn eval_is_directed(
    value: Value,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::EdgeRef(id) => Ok(Value::Bool(ctx.tx.snapshot().edge_endpoints(id).is_some())),
        Value::NodeRef(_) => data_exception("IS DIRECTED operand is not an edge", span),
        _ => data_exception("IS DIRECTED operand is not a graph element", span),
    }
}

fn eval_is_labeled(
    value: Value,
    label_expr: &LabelExpr,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::NodeRef(id) => {
            Ok(Value::Bool(ctx.tx.snapshot().node_labels(id).is_some_and(
                |labels| scan::label_matches_node(label_expr, labels),
            )))
        }
        Value::EdgeRef(id) => {
            Ok(Value::Bool(ctx.tx.snapshot().edge_label(id).is_some_and(
                |label| scan::label_matches_edge(label_expr, *label),
            )))
        }
        _ => data_exception("IS LABELED operand is not a graph element", span),
    }
}

fn eval_is_truth_value(value: Value, truth_value: TruthValue) -> Value {
    let matches_truth = match (value, truth_value) {
        (Value::Bool(true), TruthValue::True)
        | (Value::Bool(false), TruthValue::False)
        | (Value::Null, TruthValue::Unknown) => true,
        (Value::Bool(_), _) | (Value::Null, _) => false,
        _ => false,
    };
    Value::Bool(matches_truth)
}

fn eval_is_endpoint(
    operand: Value,
    value: &ValueExpr,
    source: bool,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let value = evaluate(value, binding, schema, ctx)?;
    if matches!(operand, Value::Null) || matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let Value::EdgeRef(edge_id) = operand else {
        return data_exception("endpoint predicate operand is not an edge", span);
    };
    let Value::NodeRef(node_id) = value else {
        return data_exception("endpoint predicate value is not a node", span);
    };
    Ok(Value::Bool(
        ctx.tx
            .snapshot()
            .edge_endpoints(edge_id)
            .is_some_and(|(source_id, destination_id)| {
                if source {
                    source_id == node_id
                } else {
                    destination_id == node_id
                }
            }),
    ))
}

fn value_matches_type(value: &Value, ty: &GqlType) -> bool {
    match ty {
        GqlType::String => matches!(value, Value::String(_) | Value::ExternalString(_)),
        GqlType::Uuid => matches!(value, Value::Uuid(_)),
        GqlType::Boolean => matches!(value, Value::Bool(_)),
        GqlType::Integer
        | GqlType::Int8
        | GqlType::Int16
        | GqlType::Int32
        | GqlType::Int64
        | GqlType::SmallInt
        | GqlType::BigInt => matches!(value, Value::Int(_)),
        GqlType::Int128 => matches!(value, Value::Int128(_)),
        GqlType::Uint8 | GqlType::Uint16 | GqlType::Uint32 | GqlType::Uint64 => {
            matches!(value, Value::Uint(_))
        }
        GqlType::Uint128 => matches!(value, Value::Uint128(_)),
        GqlType::Float => matches!(value, Value::Float(_) | Value::Float32(_)),
        GqlType::Float32 => matches!(value, Value::Float32(_)),
        GqlType::Float64 => matches!(value, Value::Float(_)),
        GqlType::Decimal => matches!(value, Value::Decimal(_)),
        GqlType::Bytes | GqlType::Binary | GqlType::VarBinary => matches!(value, Value::Bytes(_)),
        GqlType::ZonedDateTime => matches!(value, Value::ZonedDateTime(_)),
        GqlType::LocalDateTime => matches!(value, Value::LocalDateTime(_)),
        GqlType::Date => matches!(value, Value::Date(_)),
        GqlType::ZonedTime => matches!(value, Value::ZonedTime(_)),
        GqlType::LocalTime => matches!(value, Value::LocalTime(_)),
        GqlType::Duration => matches!(value, Value::Duration(_)),
        GqlType::Record(_) => matches!(value, Value::Record(_) | Value::RecordTyped(_)),
        GqlType::List(_) => matches!(value, Value::List(_)),
        GqlType::Path => matches!(value, Value::Path(_)),
        GqlType::GraphRef => matches!(value, Value::GraphRef(_)),
        GqlType::NodeRef => matches!(value, Value::NodeRef(_)),
        GqlType::EdgeRef => matches!(value, Value::EdgeRef(_)),
        GqlType::TableRef => matches!(value, Value::TableRef(_)),
        GqlType::Null => matches!(value, Value::Null),
        GqlType::Nothing => false,
    }
}

fn negate_predicate(value: Value, negated: bool) -> Result<Value, ExecutorError> {
    if !negated {
        return Ok(value);
    }
    Ok(match value {
        Value::Bool(value) => Value::Bool(!value),
        Value::Null => Value::Null,
        other => other,
    })
}
