//! Runtime validation for typed parameter declarations.

use std::borrow::Cow;

use selene_core::{IStr, Record, Value};

use crate::{GqlType, RecordType, SourceSpan, runtime::ExecutorError};

pub(crate) fn validate_declared_type(
    name: IStr,
    value: &Value,
    declared_type: &GqlType,
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    if value_matches_gql_type(value, declared_type) {
        return Ok(());
    }
    Err(ExecutorError::InvalidParameterType {
        name,
        expected: render_gql_type(declared_type),
        actual: value_gql_type_name(value),
        span,
    })
}

fn value_matches_gql_type(value: &Value, ty: &GqlType) -> bool {
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
        GqlType::Record(record) => value_matches_record_type(value, record),
        GqlType::List(inner) => match value {
            Value::List(values) => values
                .iter()
                .all(|value| value_matches_gql_type(value, inner)),
            _ => false,
        },
        GqlType::Path => matches!(value, Value::Path(_)),
        GqlType::GraphRef => matches!(value, Value::GraphRef(_)),
        GqlType::NodeRef => matches!(value, Value::NodeRef(_)),
        GqlType::EdgeRef => matches!(value, Value::EdgeRef(_)),
        GqlType::TableRef => matches!(value, Value::TableRef(_)),
        GqlType::Null => matches!(value, Value::Null),
        GqlType::Nothing => false,
    }
}

fn value_matches_record_type(value: &Value, record: &RecordType) -> bool {
    match record {
        RecordType::Open => matches!(value, Value::Record(_) | Value::RecordTyped(_)),
        RecordType::Closed(fields) => match value {
            Value::Record(record) => match record.as_ref() {
                Record::Open(values) => {
                    values.len() == fields.len()
                        && fields.iter().all(|(name, ty)| {
                            values
                                .iter()
                                .find(|(field, _)| field == name)
                                .is_some_and(|(_, value)| value_matches_gql_type(value, ty))
                        })
                }
                _ => false,
            },
            Value::RecordTyped(record) => {
                record.values.len() == fields.len()
                    && record
                        .values
                        .iter()
                        .zip(fields.iter())
                        .all(|(value, (_, ty))| {
                            value
                                .as_ref()
                                .is_some_and(|value| value_matches_gql_type(value, ty))
                        })
            }
            _ => false,
        },
    }
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
        GqlType::Bytes | GqlType::Binary | GqlType::VarBinary => "BYTES".into(),
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
        Value::Extended { .. } => "EXTENDED",
        Value::Null => "NULL",
        Value::Uuid(_) => "UUID",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use selene_core::{Record, Value, intern};
    use smallvec::smallvec;

    use super::*;

    fn key(value: &str) -> IStr {
        intern(value).expect("test string interns")
    }

    #[test]
    fn list_parameter_validation_checks_nested_element_types() {
        let ty = GqlType::List(Box::new(GqlType::Integer));
        assert!(value_matches_gql_type(
            &Value::List(vec![Value::Int(1), Value::Int(2)]),
            &ty
        ));
        assert!(!value_matches_gql_type(
            &Value::List(vec![Value::Int(1), Value::String(key("bad"))]),
            &ty
        ));
    }

    #[test]
    fn closed_record_parameter_validation_checks_fields() {
        let ty = GqlType::Record(RecordType::Closed(vec![(key("count"), GqlType::Integer)]));
        assert!(value_matches_gql_type(
            &Value::Record(Box::new(Record::Open(smallvec![(
                key("count"),
                Value::Int(3)
            )]))),
            &ty
        ));
        assert!(!value_matches_gql_type(
            &Value::Record(Box::new(Record::Open(smallvec![(
                key("count"),
                Value::String(key("three"))
            )]))),
            &ty
        ));
    }
}
