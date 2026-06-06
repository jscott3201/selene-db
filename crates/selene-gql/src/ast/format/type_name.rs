//! GQL type-name rendering for the read-side formatter.

use crate::GqlType;
use crate::ast::RecordType;
use crate::ast::format_ident::fmt_ident;

pub(crate) fn fmt_type(ty: &GqlType) -> String {
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
        GqlType::ZonedDateTime => "ZONED DATETIME".to_owned(),
        GqlType::LocalDateTime => "LOCAL DATETIME".to_owned(),
        GqlType::Date => "DATE".to_owned(),
        GqlType::ZonedTime => "ZONED TIME".to_owned(),
        GqlType::LocalTime => "LOCAL TIME".to_owned(),
        GqlType::Duration => "DURATION".to_owned(),
        GqlType::Vector => "VECTOR".to_owned(),
        // Recurse into the element type so `LIST<INT8>` round-trips through
        // parse-format-parse without rewriting the element type.
        GqlType::List(inner) => format!("LIST<{}>", fmt_type(inner)),
        GqlType::Path => "PATH".to_owned(),
        GqlType::Null => "NULL".to_owned(),
        GqlType::Nothing => "NOTHING".to_owned(),
        // Bare `RECORD` (GV47) carries no field structure; the closed form
        // (GV46/GV48) renders each `<field name> :: <value type>` pair so a
        // typed predicate (`IS TYPED RECORD{...}`) or `CAST(x AS RECORD{...})`
        // round-trips through parse-format-parse. `::` is the field separator
        // the grammar's `record_field_type` rule accepts (Per ISO 39075:2024
        // §18.10 <field types specification>).
        GqlType::Record(RecordType::Open) => "RECORD".to_owned(),
        GqlType::Record(RecordType::Closed(fields)) => {
            let body = fields
                .iter()
                .map(|(name, field_ty)| {
                    format!("{} :: {}", fmt_ident(name.clone()), fmt_type(field_ty))
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("RECORD{{{body}}}")
        }
        // validate_formattable rejects these AST-only reference variants before
        // read-side source formatting starts. Crate-internal diagnostics still
        // use this renderer directly, so preserve the logical type names here.
        GqlType::GraphRef => "GRAPH".to_owned(),
        GqlType::NodeRef => "NODE".to_owned(),
        GqlType::EdgeRef => "EDGE".to_owned(),
        GqlType::TableRef => "TABLE".to_owned(),
    }
}
