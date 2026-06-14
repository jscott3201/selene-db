//! Collection and record expression evaluation.
//!
//! Record literals build open records and reject duplicate field keys with a
//! data exception.

use std::collections::BTreeSet;

use selene_core::{DbString, Record, Value};
use smallvec::SmallVec;

use crate::{
    SourceSpan, ValueExpr,
    runtime::{Binding, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError},
};

use super::{binary_ops::data_exception_with, evaluate};

pub(super) fn eval_record_literal(
    fields: &[(DbString, ValueExpr)],
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let mut seen = BTreeSet::new();
    let mut values = SmallVec::<[(DbString, Value); 4]>::new();
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
pub(super) fn record_field(record: &Record, key: DbString) -> Value {
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
