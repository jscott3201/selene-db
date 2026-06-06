//! Runtime validation for typed parameter declarations.

use selene_core::{DbString, Value};

use crate::{
    GqlType, SourceSpan,
    ast::format::format_gql_type,
    runtime::{ExecutorError, value_type_match},
};

pub(crate) fn validate_declared_type(
    name: DbString,
    value: &Value,
    declared_type: &GqlType,
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    if value_type_match::value_matches_gql_type(value, declared_type) {
        return Ok(());
    }
    Err(ExecutorError::InvalidParameterType {
        name,
        expected: format_gql_type(declared_type).into(),
        actual: value_gql_type_name(value),
        span,
    })
}

fn value_gql_type_name(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "BOOLEAN",
        Value::Int(_) => "INTEGER",
        Value::Uint(_) => "UINT64",
        Value::Int128(_) => "INT128",
        Value::Uint128(_) => "UINT128",
        Value::Float(_) => "FLOAT64",
        Value::Float32(_) => "FLOAT32",
        Value::Decimal(_) => "DECIMAL",
        Value::String(_) => "STRING",
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
        Value::Extended { .. } => "EXTENDED",
        Value::Null => "NULL",
        Value::Uuid(_) => "UUID",
        Value::Vector(_) => "VECTOR",
        _ => "UNKNOWN",
    }
}
