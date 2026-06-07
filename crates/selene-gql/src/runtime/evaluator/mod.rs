//! Residual-filter expression evaluator.
//!
//! BRIEF-116 factors evaluator behavior by expression family:
//! [`binary_ops`] owns operators, [`predicates`] owns GQL predicate forms,
//! [`scalar_fns`] owns the v1.1 closed scalar-function set, [`case`] owns
//! searched `CASE`, [`collections`] owns list/record expressions, and
//! [`subquery`] owns planned expression subqueries.

mod binary_ops;
mod case;
mod cast;
mod collections;
mod identity_length_fns;
mod json_fns;
mod predicates;
mod scalar_fns;
mod string_fns;
mod subquery;
mod temporal_fns;
mod uuid_fns;

use selene_core::{EdgeId, NodeId, Value};

use crate::{
    Literal, SourceSpan, ValueExpr,
    runtime::{Binding, BindingTableSchema, EvalCtx, ExecutorError},
};
// Used only by `evaluate_for_test` below, which is itself gated to the
// test/`test-harness` surface (D21). Without the matching gate these imports
// are unused in a default-features lib build (`cargo clippy --locked`).
#[cfg(any(test, feature = "test-harness"))]
use crate::{SubqueryRegistry, analyze::ExprIdLookup, runtime::TxContext};

use self::{
    binary_ops::{eval_binary, eval_in_list, eval_unary},
    case::eval_case,
    collections::{eval_list_access, eval_record_literal, record_field},
    predicates::{eval_all_different, eval_is_check, eval_property_exists, eval_same},
    scalar_fns::eval_function_call,
    subquery::{eval_count_subquery, eval_exists, eval_value_subquery},
};

/// Evaluate a value expression against one binding-table row.
pub fn evaluate(
    expr: &ValueExpr,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    match expr {
        ValueExpr::Literal(literal) => Ok(literal_value(literal)),
        ValueExpr::Variable { name, span } => lookup_variable(name.clone(), *span, binding, schema),
        ValueExpr::PropertyAccess { target, key, span } => {
            let target = evaluate(target, binding, schema, ctx)?;
            property_access(&target, key.clone(), *span, ctx)
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
            kind,
            negated,
            span,
        } => eval_is_check(operand, kind, *negated, *span, binding, schema, ctx),
        ValueExpr::InList {
            operand,
            list,
            negated,
            span,
        } => {
            let value = evaluate(operand, binding, schema, ctx)?;
            eval_in_list(value, list, *negated, *span, binding, schema, ctx)
        }
        ValueExpr::ListLiteral { items, .. } => items
            .iter()
            .map(|item| evaluate(item, binding, schema, ctx))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::List),
        ValueExpr::Parameter {
            name,
            declared_type,
            span,
        } => resolve_parameter(name.clone(), declared_type.as_ref(), *span, ctx),
        ValueExpr::FunctionCall {
            name,
            args,
            star,
            distinct,
            span,
        } => eval_function_call(name, args, (*star, *distinct), *span, binding, schema, ctx),
        ValueExpr::Normalize { source, form, span } => {
            let value = evaluate(source, binding, schema, ctx)?;
            string_fns::eval_normalize(value, *form, *span)
        }
        ValueExpr::Trim {
            spec,
            character,
            source,
            span,
        } => {
            let source = evaluate(source, binding, schema, ctx)?;
            let character = character
                .as_deref()
                .map(|character| evaluate(character, binding, schema, ctx))
                .transpose()?;
            string_fns::eval_explicit_trim(source, character, (*spec).into(), *span)
        }
        ValueExpr::Case {
            branches,
            else_branch,
            span,
        } => eval_case(
            branches,
            else_branch.as_deref(),
            *span,
            binding,
            schema,
            ctx,
        ),
        ValueExpr::Exists { negated, span, .. } => {
            eval_exists(expr, *negated, *span, binding, schema, ctx)
        }
        ValueExpr::CountSubquery { span, .. } => {
            eval_count_subquery(expr, *span, binding, schema, ctx)
        }
        ValueExpr::ValueSubquery { span, .. } => {
            eval_value_subquery(expr, *span, binding, schema, ctx)
        }
        ValueExpr::AllDifferent { items, span } => {
            eval_all_different(items, *span, binding, schema, ctx)
        }
        ValueExpr::Same { items, span } => eval_same(items, *span, binding, schema, ctx),
        ValueExpr::PropertyExists { target, key, span } => {
            eval_property_exists(target, key.clone(), *span, binding, schema, ctx)
        }
        ValueExpr::ListAccess {
            target,
            index,
            span,
        } => eval_list_access(target, index, *span, binding, schema, ctx),
        ValueExpr::RecordLiteral { fields, span } => {
            eval_record_literal(fields, *span, binding, schema, ctx)
        }
        ValueExpr::Cast {
            value,
            target_type,
            span,
        } => {
            let evaluated = evaluate(value, binding, schema, ctx)?;
            cast::eval_cast(evaluated, target_type, *span, ctx)
        }
    }
}

/// Evaluate an expression without a plan-level subquery registry.
///
/// This preserves the public test helper surface for expression families that
/// do not require planned subqueries. Statement execution uses [`evaluate`]
/// with the owning execution plan's registries.
///
/// Gated to the test/`test-harness` surface (D21) so this scaffolding stays off
/// the always-on public API; the re-export in `runtime` carries the same gate.
#[cfg(any(test, feature = "test-harness"))]
pub fn evaluate_for_test(
    expr: &ValueExpr,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &TxContext<'_, '_>,
) -> Result<Value, ExecutorError> {
    let expr_ids = ExprIdLookup::default();
    let subqueries = SubqueryRegistry::default();
    let eval_ctx = EvalCtx {
        tx: ctx,
        expr_ids: &expr_ids,
        subqueries: &subqueries,
    };
    evaluate(expr, binding, schema, &eval_ctx)
}

fn lookup_variable(
    name: selene_core::DbString,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
) -> Result<Value, ExecutorError> {
    let Some(index) = schema
        .columns
        .iter()
        .position(|column| column.name == Some(name.clone()))
    else {
        // GA07 binder keeps pre-projection bindings visible after RETURN.
        // OrderBy evaluates against the projected schema when the TopK
        // rewrite does not apply (unbounded ORDER BY); a strict
        // InvalidReference here would break those plans. Surface
        // analyzer-fault unbound vars at bind-time instead.
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

fn resolve_parameter(
    name: selene_core::DbString,
    declared_type: Option<&crate::GqlType>,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let value = ctx
        .tx
        .parameters()
        .get(&name)
        .cloned()
        .ok_or(ExecutorError::UnboundParameter {
            name: name.clone(),
            span,
        })?;
    if let Some(declared_type) = declared_type {
        crate::runtime::parameter_type::validate_declared_type(name, &value, declared_type, span)?;
    }
    Ok(value)
}

pub(super) fn property_access(
    target: &Value,
    key: selene_core::DbString,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    match target {
        Value::Null => Ok(Value::Null),
        Value::NodeRef(id) => Ok(property_from_node(*id, &key, ctx)),
        Value::EdgeRef(id) => Ok(property_from_edge(*id, &key, ctx)),
        Value::Record(record) => Ok(record_field(record, key)),
        Value::List(items) => property_list_access(items, &key, span, ctx),
        // A non-element / non-record target (reachable when analysis types the
        // target as Dynamic, e.g. `[1,2,3].foo` or `(123).foo`) is a runtime
        // type error, not an internal-invariant break. `Value::RecordTyped`
        // stays fail-closed here (catalog-bound, no inline-name index).
        _ => Err(ExecutorError::data_exception(
            crate::runtime::DataExceptionSubclass::InvalidValueType,
            "property access target is not a node, edge, record, or list".to_owned(),
            span,
        )),
    }
}

fn property_list_access(
    items: &[Value],
    key: &selene_core::DbString,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let value = match item {
            Value::Null => Value::Null,
            Value::NodeRef(id) => property_from_node(*id, key, ctx),
            Value::EdgeRef(id) => property_from_edge(*id, key, ctx),
            Value::Record(record) => record_field(record, key.clone()),
            _ => {
                return Err(ExecutorError::data_exception(
                    crate::runtime::DataExceptionSubclass::InvalidValueType,
                    "property access list item is not a node, edge, record, or null".to_owned(),
                    span,
                ));
            }
        };
        values.push(value);
    }
    Ok(Value::List(values))
}

fn property_from_node(
    id: NodeId,
    key: &selene_core::DbString,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Value {
    ctx.tx
        .snapshot()
        .node_properties(id)
        .and_then(|properties| properties.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn property_from_edge(
    id: EdgeId,
    key: &selene_core::DbString,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Value {
    ctx.tx
        .snapshot()
        .edge_properties(id)
        .and_then(|properties| properties.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn literal_value(literal: &Literal) -> Value {
    match literal {
        Literal::Bool(value, _) => Value::Bool(*value),
        Literal::Integer(value, _) | Literal::RadixInteger(value, _, _) => Value::Int(*value),
        Literal::Float(value, _) => Value::Float(*value),
        Literal::String(value, _) => Value::String(value.clone()),
        Literal::Bytes(value, _) => Value::Bytes(value.clone()),
        Literal::Uuid(value, _) => Value::Uuid(*value),
        Literal::ZonedDateTime(value, _) => Value::ZonedDateTime(value.clone()),
        Literal::LocalDateTime(value, _) => Value::LocalDateTime(*value),
        Literal::Date(value, _) => Value::Date(*value),
        Literal::ZonedTime(value, _) => Value::ZonedTime(value.clone()),
        Literal::LocalTime(value, _) => Value::LocalTime(*value),
        Literal::Duration(value, _) => Value::Duration(value.clone()),
        Literal::Null(_) => Value::Null,
    }
}
