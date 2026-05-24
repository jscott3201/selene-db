//! GQL type-name rendering for the read-side formatter.

use crate::GqlType;

pub(super) fn fmt_type(ty: &GqlType) -> String {
    match ty {
        GqlType::String => "STRING".to_owned(),
        GqlType::Boolean => "BOOLEAN".to_owned(),
        GqlType::Integer => "INTEGER".to_owned(),
        GqlType::Float => "FLOAT".to_owned(),
        GqlType::Int8 => "INT8".to_owned(),
        GqlType::Int16 => "INT16".to_owned(),
        GqlType::Int32 => "INT32".to_owned(),
        GqlType::Int64 => "INT64".to_owned(),
        GqlType::Int128 => "INT128".to_owned(),
        GqlType::Uint8 => "UINT8".to_owned(),
        GqlType::Uint16 => "UINT16".to_owned(),
        GqlType::Uint32 => "UINT32".to_owned(),
        GqlType::Uint64 => "UINT64".to_owned(),
        GqlType::Uint128 => "UINT128".to_owned(),
        GqlType::SmallInt => "SMALLINT".to_owned(),
        GqlType::BigInt => "BIGINT".to_owned(),
        GqlType::Decimal => "DECIMAL".to_owned(),
        GqlType::Float32 => "FLOAT32".to_owned(),
        GqlType::Float64 => "FLOAT64".to_owned(),
        GqlType::Bytes => "BYTES".to_owned(),
        GqlType::Uuid => "UUID".to_owned(),
        GqlType::Binary => "BYTES".to_owned(),
        GqlType::VarBinary => "BYTES".to_owned(),
        GqlType::ZonedDateTime => "ZONED DATETIME".to_owned(),
        GqlType::LocalDateTime => "LOCAL DATETIME".to_owned(),
        GqlType::Date => "DATE".to_owned(),
        GqlType::ZonedTime => "ZONED TIME".to_owned(),
        GqlType::LocalTime => "LOCAL TIME".to_owned(),
        GqlType::Duration => "DURATION".to_owned(),
        // Recurse into the element type so `LIST<INT8>` round-trips through
        // parse-format-parse without rewriting the element type.
        GqlType::List(inner) => format!("LIST<{}>", fmt_type(inner)),
        GqlType::Path => "PATH".to_owned(),
        GqlType::Null => "NULL".to_owned(),
        GqlType::Nothing => "NOTHING".to_owned(),
        // validate_formattable rejects these AST-only variants before the
        // formatter starts. This arm remains a defensive fallback for callers
        // that bypass the public formatting entry point inside this module.
        GqlType::Record(_)
        | GqlType::GraphRef
        | GqlType::NodeRef
        | GqlType::EdgeRef
        | GqlType::TableRef => "STRING".to_owned(),
    }
}
