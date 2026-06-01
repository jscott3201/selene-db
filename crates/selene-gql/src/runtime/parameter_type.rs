//! Runtime validation for typed parameter declarations.

use std::borrow::Cow;

use selene_core::{IStr, Value};

use crate::{GqlType, SourceSpan, runtime::ExecutorError, runtime::value_type_match};

pub(crate) fn validate_declared_type(
    name: IStr,
    value: &Value,
    declared_type: &GqlType,
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    if value_type_match::value_matches_gql_type(value, declared_type) {
        return Ok(());
    }
    Err(ExecutorError::InvalidParameterType {
        name,
        expected: render_gql_type(declared_type),
        actual: value_gql_type_name(value),
        span,
    })
}

fn render_gql_type(ty: &GqlType) -> Cow<'static, str> {
    match ty {
        GqlType::String => "STRING".into(),
        GqlType::Boolean => "BOOLEAN".into(),
        GqlType::Integer => "INTEGER".into(),
        GqlType::Float => "FLOAT".into(),
        GqlType::Int8 => "INT8".into(),
        GqlType::Int16 => "INT16".into(),
        GqlType::Int32 => "INT32".into(),
        GqlType::Int64 => "INT64".into(),
        GqlType::Int128 => "INT128".into(),
        GqlType::Uint8 => "UINT8".into(),
        GqlType::Uint16 => "UINT16".into(),
        GqlType::Uint32 => "UINT32".into(),
        GqlType::Uint64 => "UINT64".into(),
        GqlType::Uint128 => "UINT128".into(),
        GqlType::SmallInt => "SMALLINT".into(),
        GqlType::BigInt => "BIGINT".into(),
        GqlType::Decimal => "DECIMAL".into(),
        GqlType::Float32 => "FLOAT32".into(),
        GqlType::Float64 => "FLOAT64".into(),
        GqlType::Bytes => "BYTES".into(),
        GqlType::Uuid => "UUID".into(),
        GqlType::ZonedDateTime => "ZONED DATETIME".into(),
        GqlType::LocalDateTime => "LOCAL DATETIME".into(),
        GqlType::Date => "DATE".into(),
        GqlType::ZonedTime => "ZONED TIME".into(),
        GqlType::LocalTime => "LOCAL TIME".into(),
        GqlType::Duration => "DURATION".into(),
        GqlType::Record(_) => "RECORD".into(),
        GqlType::List(inner) => format!("LIST<{}>", render_gql_type(inner)).into(),
        GqlType::Path => "PATH".into(),
        GqlType::GraphRef => "GRAPH".into(),
        GqlType::NodeRef => "NODE".into(),
        GqlType::EdgeRef => "EDGE".into(),
        GqlType::TableRef => "TABLE".into(),
        GqlType::Null => "NULL".into(),
        GqlType::Nothing => "NOTHING".into(),
    }
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
        _ => "UNKNOWN",
    }
}
