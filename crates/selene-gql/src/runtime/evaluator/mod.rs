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
mod predicates;
mod scalar_fns;
mod string_fns;
mod subquery;
mod temporal_fns;
mod uuid_fns;

use selene_core::{EdgeId, NodeId, Value};

use crate::{
    Literal, SourceSpan, SubqueryRegistry, ValueExpr,
    analyze::ExprIdLookup,
    runtime::{Binding, BindingTableSchema, EvalCtx, ExecutorError, TxContext},
};

use self::{
    binary_ops::{eval_binary, eval_in_list, eval_unary},
    case::eval_case,
    collections::{eval_list_access, eval_record_literal},
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
        ValueExpr::Parameter {
            name,
            declared_type,
            span,
        } => resolve_parameter(*name, declared_type.as_ref(), *span, ctx),
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
        ValueExpr::Cast {
            value,
            target_type,
            span,
        } => {
            let evaluated = evaluate(value, binding, schema, ctx)?;
            cast::eval_cast(evaluated, target_type, *span)
        }
    }
}

/// Evaluate an expression without a plan-level subquery registry.
///
/// This preserves the public test helper surface for expression families that
/// do not require planned subqueries. Statement execution uses [`evaluate`]
/// with the owning execution plan's registries.
#[allow(dead_code)]
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

fn resolve_parameter(
    name: selene_core::IStr,
    declared_type: Option<&crate::GqlType>,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let value = ctx
        .tx
        .parameters()
        .get(&name)
        .cloned()
        .ok_or(ExecutorError::UnboundParameter { name, span })?;
    if let Some(declared_type) = declared_type {
        crate::runtime::parameter_type::validate_declared_type(name, &value, declared_type, span)?;
    }
    Ok(value)
}

pub(super) fn property_access(
    target: &Value,
    key: selene_core::IStr,
    ctx: &EvalCtx<'_, '_, '_, '_>,
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

fn property_from_node(id: NodeId, key: selene_core::IStr, ctx: &EvalCtx<'_, '_, '_, '_>) -> Value {
    ctx.tx
        .snapshot()
        .node_properties(id)
        .and_then(|properties| properties.get(&key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn property_from_edge(id: EdgeId, key: selene_core::IStr, ctx: &EvalCtx<'_, '_, '_, '_>) -> Value {
    ctx.tx
        .snapshot()
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
        Literal::Uuid(value, _) => Value::Uuid(*value),
        Literal::Null(_) => Value::Null,
    }
}
