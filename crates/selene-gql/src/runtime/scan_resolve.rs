//! Parameter-aware [`IndexKey`] resolution for indexed-scan probes.
//!
//! Houses the runtime support that bridges the plan-time `IndexKey` IR
//! (BRIEF-154 §B.1) to the snapshot's typed-index probe surface. Lifted out
//! of `runtime/scan.rs` to keep that file under the 700 LOC cap; logic and
//! cross-references unchanged.

use selene_core::{IStr, Value};

use crate::{
    IndexKey, IndexKind, Literal, SourceSpan, TypedIndexBounds,
    runtime::{EvalCtx, ExecutorError, parameter_type},
};

/// Result of resolving an [`IndexKey`] against bound parameters.
pub(super) enum IndexKeyOutcome {
    /// Concrete probe value.
    Value(Value),
    /// Probe known to return zero rows because the parameter binding cannot
    /// match any indexed row. Used for NULL bindings (3VL parity with inline
    /// `WHERE n.x = NULL`, per BRIEF-154 §B.3 F5) and for `Value::ExternalString`
    /// bindings against `STRING` indexes whose content has never been admitted
    /// to the global `IStr` pool (BRIEF-153 read-side discipline, §B.3 F4).
    EmptyResult,
}

/// A typed-index probe bound, with every key resolved to a concrete
/// [`Value`]. Built once at scan-entry by [`resolve_bounds`] so the index
/// probe and the linear-fallback predicate evaluator never have to re-resolve
/// parameter slots per row.
pub(super) enum ResolvedBounds {
    Equality(Value),
    GreaterThan(Value),
    GreaterEqual(Value),
    LessThan(Value),
    LessEqual(Value),
    Range {
        lo: Value,
        lo_inclusive: bool,
        hi: Value,
        hi_inclusive: bool,
    },
}

/// Resolve a single [`IndexKey`] into a probe value or an empty-result
/// short-circuit (BRIEF-154 §B.3).
///
/// `IndexKey::Literal` short-circuits straight through [`literal_to_value`].
/// `IndexKey::Parameter` consults `ctx.tx.parameters()`:
/// - **Unbound name** → [`ExecutorError::UnboundParameter`] (loud).
/// - **`Value::Null` binding** → [`IndexKeyOutcome::EmptyResult`] (3VL parity).
/// - **`Value::ExternalString` against an `IndexKind::String`** → look up via
///   [`selene_core::lookup`]; `Some(istr)` coerces to `Value::String(istr)`,
///   `None` returns `EmptyResult` without admitting the string to the pool.
/// - **Declared type set** → [`parameter_type::validate_declared_type`] (typed
///   parameter contract; mismatch errors `InvalidParameterType` / `22G03`).
/// - **`IndexKind` mismatch on the resolved Value** → loud
///   `InvalidParameterType` so the storage drift does not silently surface as
///   "zero rows" (BRIEF-154 §B.3 F12).
pub(super) fn resolve_index_key(
    key: &IndexKey,
    expected_kind: IndexKind,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<IndexKeyOutcome, ExecutorError> {
    match key {
        IndexKey::Literal(literal) => Ok(IndexKeyOutcome::Value(literal_to_value(literal))),
        IndexKey::Parameter {
            name,
            declared_type,
            span,
        } => {
            let raw =
                ctx.tx
                    .parameters()
                    .get(name)
                    .cloned()
                    .ok_or(ExecutorError::UnboundParameter {
                        name: *name,
                        span: *span,
                    })?;
            if matches!(raw, Value::Null) {
                return Ok(IndexKeyOutcome::EmptyResult);
            }
            // BRIEF-153 read-side carve-out for STRING-indexed columns:
            // ExternalString must NOT enter the global pool from the
            // read path. If the content is already admitted, coerce to
            // a poolable Value::String for downstream variant-strict
            // probes; otherwise empty out without admitting.
            if matches!(expected_kind, IndexKind::String)
                && let Value::ExternalString(arc) = &raw
            {
                return Ok(match selene_core::lookup(arc.as_ref()) {
                    Some(istr) => IndexKeyOutcome::Value(Value::String(istr)),
                    None => IndexKeyOutcome::EmptyResult,
                });
            }
            if let Some(declared) = declared_type {
                parameter_type::validate_declared_type(*name, &raw, declared, *span)?;
            }
            check_value_index_kind(&raw, expected_kind, *name, *span)?;
            Ok(IndexKeyOutcome::Value(raw))
        }
    }
}

/// Pre-resolve every [`IndexKey`] in `bounds` against the bound parameters.
///
/// Returns `Ok(None)` when any key resolves to [`IndexKeyOutcome::EmptyResult`]
/// — the entire probe short-circuits to an empty result. Errors propagate
/// (unbound parameter, kind mismatch, typed-param mismatch).
pub(super) fn resolve_bounds(
    bounds: &TypedIndexBounds,
    expected_kind: IndexKind,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Option<ResolvedBounds>, ExecutorError> {
    let resolve_one = |key: &IndexKey| -> Result<Option<Value>, ExecutorError> {
        match resolve_index_key(key, expected_kind, ctx)? {
            IndexKeyOutcome::Value(value) => Ok(Some(value)),
            IndexKeyOutcome::EmptyResult => Ok(None),
        }
    };
    Ok(Some(match bounds {
        TypedIndexBounds::Equality(key) => {
            let Some(value) = resolve_one(key)? else {
                return Ok(None);
            };
            ResolvedBounds::Equality(value)
        }
        TypedIndexBounds::GreaterThan(key) => {
            let Some(value) = resolve_one(key)? else {
                return Ok(None);
            };
            ResolvedBounds::GreaterThan(value)
        }
        TypedIndexBounds::GreaterEqual(key) => {
            let Some(value) = resolve_one(key)? else {
                return Ok(None);
            };
            ResolvedBounds::GreaterEqual(value)
        }
        TypedIndexBounds::LessThan(key) => {
            let Some(value) = resolve_one(key)? else {
                return Ok(None);
            };
            ResolvedBounds::LessThan(value)
        }
        TypedIndexBounds::LessEqual(key) => {
            let Some(value) = resolve_one(key)? else {
                return Ok(None);
            };
            ResolvedBounds::LessEqual(value)
        }
        TypedIndexBounds::Range {
            lo,
            lo_inclusive,
            hi,
            hi_inclusive,
        } => {
            let Some(lo_value) = resolve_one(lo)? else {
                return Ok(None);
            };
            let Some(hi_value) = resolve_one(hi)? else {
                return Ok(None);
            };
            ResolvedBounds::Range {
                lo: lo_value,
                lo_inclusive: *lo_inclusive,
                hi: hi_value,
                hi_inclusive: *hi_inclusive,
            }
        }
    }))
}

/// Lower a planner [`Literal`] to a runtime [`Value`].
pub(super) fn literal_to_value(literal: &Literal) -> Value {
    match literal {
        Literal::Bool(value, _) => Value::Bool(*value),
        Literal::Integer(value, _) => Value::Int(*value),
        Literal::Float(value, _) => Value::Float(*value),
        Literal::String(value, _) => Value::String(*value),
        Literal::Uuid(value, _) => Value::Uuid(*value),
        Literal::Null(_) => Value::Null,
    }
}

/// Loud check that a resolved parameter value matches the targeted
/// `IndexKind`, identifying the offending parameter by name in the error
/// message (BRIEF-154 §B.3 F12).
fn check_value_index_kind(
    value: &Value,
    expected: IndexKind,
    name: IStr,
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    let matches = match expected {
        IndexKind::Integer => matches!(value, Value::Int(_)),
        IndexKind::Float => matches!(value, Value::Float(_)),
        IndexKind::String => matches!(value, Value::String(_) | Value::ExternalString(_)),
        IndexKind::Date => matches!(value, Value::Date(_)),
        IndexKind::LocalDateTime => matches!(value, Value::LocalDateTime(_)),
        IndexKind::Uuid => matches!(value, Value::Uuid(_)),
    };
    if matches {
        return Ok(());
    }
    Err(ExecutorError::InvalidParameterType {
        name,
        expected: index_kind_label(expected).into(),
        actual: value_kind_label(value),
        span,
    })
}

fn index_kind_label(kind: IndexKind) -> &'static str {
    match kind {
        IndexKind::Integer => "INTEGER",
        IndexKind::Float => "FLOAT",
        IndexKind::String => "STRING",
        IndexKind::Date => "DATE",
        IndexKind::LocalDateTime => "LOCAL DATETIME",
        IndexKind::Uuid => "UUID",
    }
}

fn value_kind_label(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "BOOLEAN",
        Value::Int(_) => "INTEGER",
        Value::Uint(_) => "UINT64",
        Value::Int128(_) => "INT128",
        Value::Uint128(_) => "UINT128",
        Value::Float(_) => "FLOAT64",
        Value::Float32(_) => "FLOAT32",
        Value::Decimal(_) => "DECIMAL",
        Value::String(_) | Value::ExternalString(_) => "STRING",
        Value::Bytes(_) => "BYTES",
        Value::List(_) => "LIST",
        Value::Record(_) | Value::RecordTyped(_) => "RECORD",
        Value::Path(_) => "PATH",
        Value::NodeRef(_) => "NODE",
        Value::EdgeRef(_) => "EDGE",
        Value::GraphRef(_) => "GRAPH",
        Value::TableRef(_) => "TABLE",
        Value::ZonedDateTime(_) => "ZONED DATETIME",
        Value::LocalDateTime(_) => "LOCAL DATETIME",
        Value::Date(_) => "DATE",
        Value::ZonedTime(_) => "ZONED TIME",
        Value::LocalTime(_) => "LOCAL TIME",
        Value::Duration(_) => "DURATION",
        Value::Uuid(_) => "UUID",
        Value::Extended { .. } => "EXTENDED",
        Value::Null => "NULL",
        _ => "UNKNOWN",
    }
}
