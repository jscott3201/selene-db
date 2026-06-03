use crate::{
    LimitAmount, SourceSpan,
    runtime::{BindingTable, ExecutorError, TxContext, parameter_type},
};
use selene_core::{IStr, Value};

pub(super) fn execute(
    offset: &LimitAmount,
    count: &LimitAmount,
    table: BindingTable,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let offset = resolve_amount(offset, ctx)?;
    let count = resolve_amount(count, ctx)?;
    let (schema, rows) = table.into_parts();
    ctx.check_cancellation()?;
    let start = u64_to_bounded_usize(offset, rows.len());
    let end = start.saturating_add(u64_to_bounded_usize(count, rows.len() - start));
    Ok(BindingTable::new(schema, rows[start..end].to_vec()))
}

pub(super) fn resolve_amount(
    amount: &LimitAmount,
    ctx: &TxContext<'_, '_>,
) -> Result<u64, ExecutorError> {
    match amount {
        LimitAmount::Literal(value) => Ok(*value),
        LimitAmount::Parameter {
            name,
            declared_type,
            span,
        } => match ctx.parameters().get(name) {
            Some(value) => {
                if let Some(declared_type) = declared_type {
                    parameter_type::validate_declared_type(
                        name.clone(),
                        value,
                        declared_type,
                        *span,
                    )?;
                }
                parameter_amount(name.clone(), *span, value)
            }
            None => Err(ExecutorError::UnboundParameter {
                name: name.clone(),
                span: *span,
            }),
        },
    }
}

fn parameter_amount(name: IStr, span: SourceSpan, value: &Value) -> Result<u64, ExecutorError> {
    match value {
        Value::Int(value) if *value >= 0 => Ok(*value as u64),
        Value::Int(_) => Err(invalid_parameter_type(name, span, "negative integer")),
        Value::Uint(value) => Ok(*value),
        _ => Err(invalid_parameter_type(name, span, value_type_name(value))),
    }
}

fn invalid_parameter_type(name: IStr, span: SourceSpan, actual: &'static str) -> ExecutorError {
    ExecutorError::InvalidParameterType {
        name,
        expected: "non-negative integer".into(),
        actual,
        span,
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "boolean",
        Value::Int(_) => "integer",
        Value::Uint(_) => "unsigned integer",
        Value::Int128(_) => "128-bit integer",
        Value::Uint128(_) => "unsigned 128-bit integer",
        Value::Float(_) => "float",
        Value::Float32(_) => "float32",
        Value::Decimal(_) => "decimal",
        Value::String(_) => "string",
        Value::Bytes(_) => "bytes",
        Value::List(_) => "list",
        Value::Record(_) | Value::RecordTyped(_) => "record",
        Value::Path(_) => "path",
        Value::NodeRef(_) => "node reference",
        Value::EdgeRef(_) => "edge reference",
        Value::GraphRef(_) => "graph reference",
        Value::TableRef(_) => "table reference",
        Value::ZonedDateTime(_) => "zoned datetime",
        Value::LocalDateTime(_) => "local datetime",
        Value::Date(_) => "date",
        Value::ZonedTime(_) => "zoned time",
        Value::LocalTime(_) => "local time",
        Value::Duration(_) => "duration",
        Value::Extended { .. } => "extended",
        Value::Null => "null",
        Value::Uuid(_) => "uuid",
        Value::Vector(_) => "vector",
        _ => "unknown",
    }
}

pub(super) fn u64_to_bounded_usize(value: u64, upper_bound: usize) -> usize {
    usize::try_from(value)
        .unwrap_or(usize::MAX)
        .min(upper_bound)
}
