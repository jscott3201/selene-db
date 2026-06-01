//! Collection and record expression evaluation.
//!
//! List access uses 1-based ordinals per ISO/IEC 39075:2024 §20.16 (`list[1]`
//! is the first element); ordinals outside `1..=cardinality` — including `0` and
//! negatives — return `NULL`. ISO defines no list-subscript operator, so this is
//! a selene-db vendor construct flagged as `IM_LIST_SUBSCRIPT` (clause 24.6).
//! Record literals build open records and reject duplicate field keys with a
//! data exception.

use std::collections::BTreeSet;

use selene_core::{IStr, Record, Value};
use smallvec::SmallVec;

use crate::{
    SourceSpan, ValueExpr,
    runtime::{Binding, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError},
};

use super::{
    binary_ops::{data_exception, data_exception_with},
    evaluate,
};

pub(super) fn eval_list_access(
    target: &ValueExpr,
    index: &ValueExpr,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let target = evaluate(target, binding, schema, ctx)?;
    let index = evaluate(index, binding, schema, ctx)?;
    if matches!(target, Value::Null) || matches!(index, Value::Null) {
        return Ok(Value::Null);
    }
    let Value::List(values) = target else {
        return data_exception("list access target is not a list", span);
    };
    // ISO §20.16 fixes list ordinals as 1-based (`list[1]` is the first
    // element). ISO has no subscript operator, so this is the selene-db vendor
    // `IM_LIST_SUBSCRIPT` construct (clause 24.6); any ordinal outside
    // `1..=cardinality` — 0, negative, beyond the end, or out of `usize` range —
    // yields NULL rather than the §20.16 list-element error (22G0C).
    let zero_based = match index {
        Value::Int(index) if index >= 1 => usize::try_from(index - 1).ok(),
        Value::Uint(index) if index != 0 => usize::try_from(index - 1).ok(),
        Value::Int(_) | Value::Uint(_) => None,
        _ => {
            return data_exception("list access index is not an integer", span);
        }
    };
    Ok(zero_based
        .and_then(|index| values.get(index).cloned())
        .unwrap_or(Value::Null))
}

pub(super) fn eval_record_literal(
    fields: &[(IStr, ValueExpr)],
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let mut seen = BTreeSet::new();
    let mut values = SmallVec::<[(IStr, Value); 4]>::new();
    for (key, expr) in fields {
        if !seen.insert(key.clone()) {
            return data_exception_with(
                DataExceptionSubclass::RecordDataFieldUnassignable,
                format!("duplicate record field: {}", key.as_str()),
                span,
            );
        }
        values.push((key.clone(), evaluate(expr, binding, schema, ctx)?));
    }
    Ok(Value::Record(Box::new(Record::Open(values))))
}

/// Reads a field from an open record value by name.
///
/// Per ISO/IEC 39075:2024 clause 20.11 `<property reference>` GR3(b)/(d)(i): a
/// named field yields its value; a field absent from an open record yields
/// `NULL` (the open-record property-reference declared type is the nullable
/// open dynamic union type, SR2(b)).
pub(super) fn record_field(record: &Record, key: IStr) -> Value {
    match record {
        Record::Open(fields) => fields
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Null),
        // `Record` is `#[non_exhaustive]`; closed/typed records resolve fields
        // positionally via the graph-type catalog (deferred to the typed-RECORD
        // brief), so they are not field-addressable through this open-record path.
        _ => Value::Null,
    }
}
