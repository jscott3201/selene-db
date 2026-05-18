//! Residual-filter expression evaluator.

mod binary_ops;
mod case;
mod collections;
mod predicates;
mod scalar_fns;

use selene_core::{EdgeId, NodeId, Value};

use crate::{
    Literal, SourceSpan, ValueExpr,
    runtime::{Binding, BindingTableSchema, ExecutorError, TxContext},
};

use self::{
    binary_ops::{eval_binary, eval_in_list, eval_unary},
    case::eval_case,
    collections::{eval_list_access, eval_record_literal},
    predicates::{
        eval_all_different, eval_between, eval_is_check, eval_like, eval_property_exists, eval_same,
    },
    scalar_fns::eval_function_call,
};

/// Evaluate a value expression against one binding-table row.
pub fn evaluate(
    expr: &ValueExpr,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &TxContext<'_, '_>,
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
        ValueExpr::Parameter { name, span } => {
            ctx.parameters()
                .get(name)
                .cloned()
                .ok_or(ExecutorError::UnboundParameter {
                    name: *name,
                    span: *span,
                })
        }
        ValueExpr::FunctionCall {
            name,
            args,
            star,
            distinct,
            span,
        } => eval_function_call(name, args, *star, *distinct, *span, binding, schema, ctx),
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
        ValueExpr::Exists { .. } => Err(ExecutorError::ImplementationDefined {
            // BRIEF-116b owns planned-IR support for expression subqueries.
            detail: "EXISTS subquery evaluation not implemented",
        }),
        ValueExpr::CountSubquery { .. } => Err(ExecutorError::ImplementationDefined {
            // BRIEF-116b owns planned-IR support for expression subqueries.
            detail: "COUNT subquery evaluation not implemented",
        }),
        ValueExpr::Like {
            operand,
            pattern,
            negated,
            span,
        } => eval_like(operand, pattern, *negated, *span, binding, schema, ctx),
        ValueExpr::Between {
            operand,
            low,
            high,
            negated,
            span,
        } => eval_between(operand, low, high, *negated, *span, binding, schema, ctx),
        ValueExpr::AllDifferent { items, span } => {
            eval_all_different(items, *span, binding, schema, ctx)
        }
        ValueExpr::Same { items, span } => eval_same(items, *span, binding, schema, ctx),
        ValueExpr::PropertyExists { target, key, span } => {
            eval_property_exists(target, *key, *span, binding, schema, ctx)
        }
        ValueExpr::ListAccess {
            target,
            index,
            span,
        } => eval_list_access(target, index, *span, binding, schema, ctx),
        ValueExpr::RecordLiteral { fields, span } => {
            eval_record_literal(fields, *span, binding, schema, ctx)
        }
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

pub(super) fn property_access(
    target: &Value,
    key: selene_core::IStr,
    ctx: &TxContext<'_, '_>,
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

fn property_from_node(id: NodeId, key: selene_core::IStr, ctx: &TxContext<'_, '_>) -> Value {
    ctx.snapshot()
        .node_properties(id)
        .and_then(|properties| properties.get(&key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn property_from_edge(id: EdgeId, key: selene_core::IStr, ctx: &TxContext<'_, '_>) -> Value {
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
