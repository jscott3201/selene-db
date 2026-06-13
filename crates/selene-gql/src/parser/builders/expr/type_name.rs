//! GQL type-name builders.

use pest::iterators::Pair;
use selene_core::feature_register::FeatureId;

use crate::{
    ast::{
        ByteStringType, ByteStringTypeForm, CharacterStringType, CharacterStringTypeForm,
        DecimalType, GqlType, MAX_BYTE_STRING_TYPE_LENGTH, MAX_CHARACTER_STRING_TYPE_LENGTH,
        MAX_DECIMAL_PRECISION, MAX_DECIMAL_SCALE, RecordType, SourceSpan,
    },
    error::ParserError,
    parser::MAX_NESTING_DEPTH,
};

use super::Rule;
use crate::parser::builders::{
    db_string_pair, keyword_starts_with, keyword_tokens_eq, not_implemented, span, unexpected_pair,
};

pub(super) fn build_type_name(pair: Pair<'_, Rule>) -> Result<GqlType, ParserError> {
    build_type_name_with_depth(pair, 0)
}

fn build_type_name_with_depth(pair: Pair<'_, Rule>, depth: u32) -> Result<GqlType, ParserError> {
    debug_assert!(matches!(
        pair.as_rule(),
        Rule::type_name | Rule::type_name_base
    ));
    let source_span = span(&pair);
    if depth > MAX_NESTING_DEPTH {
        return Err(ParserError::NestingLimitExceeded {
            limit: MAX_NESTING_DEPTH,
            span: source_span,
        });
    }

    if pair.as_rule() == Rule::type_name {
        let mut children = pair.into_inner();
        let base = children.next().ok_or_else(|| {
            ParserError::syntax("type name is missing base type", source_span, None)
        })?;
        let has_not_null = children.any(|child| child.as_rule() == Rule::type_not_null);
        let ty = build_type_name_with_depth(base, depth)?;
        return Ok(if has_not_null {
            GqlType::NotNull(Box::new(ty))
        } else {
            ty
        });
    }

    // Match the raw token sequence case- and whitespace-insensitively rather
    // than building an upper-cased, whitespace-normalized `String`: the type
    // name (and every nested LIST/RECORD level) is compared allocation-free.
    let text = pair.as_str();
    if let Some(integer_precision) = pair.clone().into_inner().find(|child| {
        matches!(
            child.as_rule(),
            Rule::signed_integer_precision_type | Rule::unsigned_integer_precision_type
        )
    }) {
        return build_integer_precision_type_name(integer_precision, source_span);
    }
    if let Some(float_precision) = pair
        .clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::float_precision_type)
    {
        return build_float_precision_type_name(float_precision, source_span);
    }
    if let Some(decimal_precision) = pair
        .clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::decimal_precision_type)
    {
        return build_decimal_precision_type_name(decimal_precision, source_span);
    }
    if keyword_tokens_eq(text, &["FLOAT16"]) {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV20,
            display_name: "16 bit floating point numbers",
            span: source_span,
            hint: "FLOAT16 is outside the selene-db v1.0 claim list; use FLOAT32 or FLOAT64",
        });
    }
    if keyword_tokens_eq(text, &["UINT256"]) || keyword_tokens_eq(text, &["UNSIGNED", "INTEGER256"])
    {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV15,
            display_name: "256 bit unsigned integer numbers",
            span: source_span,
            hint: "UINT256 is outside the selene-db v1.0 claim list",
        });
    }
    if keyword_tokens_eq(text, &["INT256"])
        || keyword_tokens_eq(text, &["INTEGER256"])
        || keyword_tokens_eq(text, &["SIGNED", "INTEGER256"])
    {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV16,
            display_name: "256 bit signed integer numbers",
            span: source_span,
            hint: "INT256 is outside the selene-db v1.0 claim list",
        });
    }
    if keyword_tokens_eq(text, &["FLOAT128"]) {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV25,
            display_name: "128 bit floating point numbers",
            span: source_span,
            hint: "FLOAT128 is outside the selene-db v1.0 claim list",
        });
    }
    if keyword_tokens_eq(text, &["FLOAT256"]) {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV26,
            display_name: "256 bit floating point numbers",
            span: source_span,
            hint: "FLOAT256 is outside the selene-db v1.0 claim list",
        });
    }
    if keyword_starts_with(text, "BYTES")
        || keyword_starts_with(text, "BINARY")
        || keyword_starts_with(text, "VARBINARY")
    {
        return build_byte_string_type_name(text, source_span);
    }
    if keyword_starts_with(text, "STRING")
        || keyword_starts_with(text, "CHAR")
        || keyword_starts_with(text, "VARCHAR")
    {
        return build_character_string_type_name(text, source_span);
    }
    if keyword_starts_with(text, "DURATION") {
        return build_duration_type_name(pair);
    }
    if keyword_starts_with(text, "LIST") {
        let inner = pair
            .into_inner()
            .find(|child| child.as_rule() == Rule::type_name)
            .ok_or_else(|| {
                ParserError::syntax("LIST type is missing element type", source_span, None)
            })?;
        return Ok(GqlType::List(Box::new(build_type_name_with_depth(
            inner,
            depth + 1,
        )?)));
    }

    if keyword_starts_with(text, "RECORD") {
        // Per ISO 39075:2024 section18.9 <record type> / <field types specification> +
        // section18.10 <field type>: a braced `RECORD { a :: INT }` is a closed record
        // type; bare `RECORD` (no fields) is the open record type. Field names are
        // user-controlled; construction applies the per-string byte cap (IL013).
        let mut fields = Vec::new();
        for field in pair
            .into_inner()
            .filter(|child| child.as_rule() == Rule::record_field_type)
        {
            let field_span = span(&field);
            let mut children = field.into_inner();
            let name_pair = children.next().ok_or_else(|| {
                ParserError::syntax("record field type is missing name", field_span, None)
            })?;
            let type_pair = children.next().ok_or_else(|| {
                ParserError::syntax("record field type is missing type", field_span, None)
            })?;
            let name = db_string_pair(name_pair)?;
            if fields
                .iter()
                .any(|(existing_name, _)| existing_name == &name)
            {
                return Err(ParserError::syntax(
                    format!("duplicate record field type name: {}", name.as_str()),
                    field_span,
                    Some("each closed RECORD type field name must be declared once".into()),
                ));
            }
            fields.push((name, build_type_name_with_depth(type_pair, depth + 1)?));
        }
        return Ok(if fields.is_empty() {
            GqlType::Record(RecordType::Open)
        } else {
            GqlType::Record(RecordType::Closed(fields))
        });
    }

    // Scalar / reference type names, matched token-wise (case- and
    // whitespace-insensitive) against their canonical spelling. The two-token
    // temporal spellings (`ZONED DATETIME`, ...) keep their multi-token match.
    const SCALAR_TYPES: &[(&[&str], GqlType)] = &[
        (&["BOOLEAN"], GqlType::Boolean),
        (&["BOOL"], GqlType::Boolean),
        (&["SIGNED", "SMALL", "INTEGER"], GqlType::SmallInt),
        (&["SIGNED", "BIG", "INTEGER"], GqlType::BigInt),
        (&["SIGNED", "INTEGER8"], GqlType::Int8),
        (&["SIGNED", "INTEGER16"], GqlType::Int16),
        (&["SIGNED", "INTEGER32"], GqlType::Int32),
        (&["SIGNED", "INTEGER64"], GqlType::Int64),
        (&["SIGNED", "INTEGER128"], GqlType::Int128),
        (&["SIGNED", "INTEGER"], GqlType::Integer),
        (&["UNSIGNED", "SMALL", "INTEGER"], GqlType::USmallInt),
        (&["UNSIGNED", "BIG", "INTEGER"], GqlType::UBigInt),
        (&["UNSIGNED", "INTEGER8"], GqlType::Uint8),
        (&["UNSIGNED", "INTEGER16"], GqlType::Uint16),
        (&["UNSIGNED", "INTEGER32"], GqlType::Uint32),
        (&["UNSIGNED", "INTEGER64"], GqlType::Uint64),
        (&["UNSIGNED", "INTEGER128"], GqlType::Uint128),
        (&["UNSIGNED", "INTEGER"], GqlType::Uint),
        (&["BIG", "INTEGER"], GqlType::BigInt),
        (&["SMALL", "INTEGER"], GqlType::SmallInt),
        (&["INTEGER8"], GqlType::Int8),
        (&["INTEGER16"], GqlType::Int16),
        (&["INTEGER32"], GqlType::Int32),
        (&["INTEGER64"], GqlType::Int64),
        (&["INTEGER128"], GqlType::Int128),
        (&["INTEGER"], GqlType::Integer),
        (&["INT"], GqlType::Integer),
        (&["INT8"], GqlType::Int8),
        (&["INT16"], GqlType::Int16),
        (&["INT32"], GqlType::Int32),
        (&["INT64"], GqlType::Int64),
        (&["INT128"], GqlType::Int128),
        (&["SMALLINT"], GqlType::SmallInt),
        (&["BIGINT"], GqlType::BigInt),
        (&["UINT"], GqlType::Uint),
        (&["UINT64"], GqlType::Uint64),
        (&["UINT8"], GqlType::Uint8),
        (&["UINT16"], GqlType::Uint16),
        (&["UINT32"], GqlType::Uint32),
        (&["UINT128"], GqlType::Uint128),
        (&["USMALLINT"], GqlType::USmallInt),
        (&["UBIGINT"], GqlType::UBigInt),
        (&["FLOAT"], GqlType::Float),
        (&["DECIMAL"], GqlType::Decimal),
        (&["DEC"], GqlType::Decimal),
        (&["FLOAT32"], GqlType::Float32),
        (&["FLOAT64"], GqlType::Float64),
        (&["REAL"], GqlType::Real),
        (&["DOUBLE"], GqlType::Double),
        (&["DOUBLE", "PRECISION"], GqlType::Double),
        (&["UUID"], GqlType::Uuid),
        (&["JSON"], GqlType::Json),
        (&["VECTOR"], GqlType::Vector),
        (&["BYTEA"], GqlType::Bytes),
        (
            &["TIMESTAMP", "WITH", "TIME", "ZONE"],
            GqlType::ZonedDateTime,
        ),
        (
            &["TIMESTAMP", "WITHOUT", "TIME", "ZONE"],
            GqlType::LocalDateTime,
        ),
        (&["TIMESTAMP"], GqlType::LocalDateTime),
        (&["TIME", "WITH", "TIME", "ZONE"], GqlType::ZonedTime),
        (&["TIME", "WITHOUT", "TIME", "ZONE"], GqlType::LocalTime),
        (&["ZONED", "DATETIME"], GqlType::ZonedDateTime),
        (&["LOCAL", "DATETIME"], GqlType::LocalDateTime),
        (&["ZONED", "TIME"], GqlType::ZonedTime),
        (&["LOCAL", "TIME"], GqlType::LocalTime),
        (&["DATE"], GqlType::Date),
        (&["PATH"], GqlType::Path),
        (&["NULL"], GqlType::Null),
        (&["NOTHING"], GqlType::Nothing),
    ];
    for (tokens, ty) in SCALAR_TYPES {
        if keyword_tokens_eq(text, tokens) {
            return Ok(ty.clone());
        }
    }
    Err(not_implemented(
        &pair,
        "this GQL type constructor is not yet supported",
    ))
}

fn build_character_string_type_name(text: &str, span: SourceSpan) -> Result<GqlType, ParserError> {
    if !text.contains('(') {
        if keyword_tokens_eq(text, &["CHAR"]) {
            return character_string_type(1, 1, CharacterStringTypeForm::CharFixed, span);
        }
        return Ok(GqlType::String);
    }
    let lengths = parse_string_type_lengths(text, span, "character string")?;
    if keyword_starts_with(text, "CHAR") {
        let [length] = string_type_single_length(&lengths, span, "character string")?;
        return character_string_type(length, length, CharacterStringTypeForm::CharFixed, span);
    }
    if keyword_starts_with(text, "VARCHAR") {
        let [max] = string_type_single_length(&lengths, span, "character string")?;
        return character_string_type(0, max, CharacterStringTypeForm::VarcharMax, span);
    }
    match lengths.as_slice() {
        [max] => character_string_type(0, *max, CharacterStringTypeForm::StringMax, span),
        [min, max] => {
            character_string_type(*min, *max, CharacterStringTypeForm::StringMinMax, span)
        }
        _ => Err(ParserError::syntax(
            "character string type expects one or two length bounds",
            span,
            None,
        )),
    }
}

fn build_integer_precision_type_name(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<GqlType, ParserError> {
    let precision_pair = pair
        .clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::numeric_type_precision)
        .ok_or_else(|| {
            ParserError::syntax("integer type is missing precision", source_span, None)
        })?;
    let precision = parse_numeric_type_precision(precision_pair.as_str(), span(&precision_pair))?;
    match pair.as_rule() {
        Rule::signed_integer_precision_type => {
            signed_integer_precision_type(precision, source_span)
        }
        Rule::unsigned_integer_precision_type => {
            unsigned_integer_precision_type(precision, source_span)
        }
        _ => Err(unexpected_pair(pair, "expected integer precision type")),
    }
}

fn parse_numeric_type_precision(text: &str, span: SourceSpan) -> Result<u16, ParserError> {
    let normalized = text.replace('_', "");
    let precision = normalized.parse::<u16>().map_err(|_err| {
        ParserError::syntax(
            "numeric type precision exceeds the implementation-defined maximum",
            span,
            Some(
                "selene-db currently supports integer precision through INT128/UINT128 and floating precision/scale through FLOAT64"
                    .into(),
            ),
        )
    })?;
    if precision == 0 {
        return Err(ParserError::syntax(
            "numeric type precision must be greater than or equal to 1",
            span,
            None,
        ));
    }
    Ok(precision)
}

fn parse_numeric_type_scale(text: &str, span: SourceSpan) -> Result<u16, ParserError> {
    let normalized = text.replace('_', "");
    normalized.parse::<u16>().map_err(|_err| {
        ParserError::syntax(
            "numeric type scale exceeds the implementation-defined maximum",
            span,
            Some("selene-db currently supports approximate numeric scale through FLOAT64".into()),
        )
    })
}

fn build_float_precision_type_name(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<GqlType, ParserError> {
    let mut children = pair.clone().into_inner();
    let precision_pair = children
        .find(|child| child.as_rule() == Rule::numeric_type_precision)
        .ok_or_else(|| ParserError::syntax("FLOAT type is missing precision", source_span, None))?;
    let precision = parse_numeric_type_precision(precision_pair.as_str(), span(&precision_pair))?;
    let scale = pair
        .clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::numeric_type_scale)
        .map(|scale_pair| parse_numeric_type_scale(scale_pair.as_str(), span(&scale_pair)))
        .transpose()?;
    if let Some(scale) = scale
        && scale > precision
    {
        return Err(ParserError::syntax(
            "numeric type scale must be less than or equal to precision",
            source_span,
            None,
        ));
    }
    float_precision_type(precision, scale, source_span)
}

fn build_decimal_precision_type_name(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<GqlType, ParserError> {
    let mut children = pair.clone().into_inner();
    let precision_pair = children
        .find(|child| child.as_rule() == Rule::numeric_type_precision)
        .ok_or_else(|| {
            ParserError::syntax("DECIMAL type is missing precision", source_span, None)
        })?;
    let precision = parse_numeric_type_precision(precision_pair.as_str(), span(&precision_pair))?;
    let scale = pair
        .clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::numeric_type_scale)
        .map(|scale_pair| parse_numeric_type_scale(scale_pair.as_str(), span(&scale_pair)))
        .transpose()?
        .unwrap_or(0);
    if scale > precision {
        return Err(ParserError::syntax(
            "numeric type scale must be less than or equal to precision",
            source_span,
            None,
        ));
    }
    decimal_precision_type(precision, scale, source_span)
}

fn decimal_precision_type(
    precision: u16,
    scale: u16,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
    DecimalType::new(precision, scale)
        .map(GqlType::DecimalExact)
        .ok_or_else(|| {
            ParserError::syntax(
                "DECIMAL precision or scale exceeds the implementation-defined maximum",
                span,
                Some(format!(
                    "selene-db currently supports DECIMAL precision up to {MAX_DECIMAL_PRECISION} digits and scale up to {MAX_DECIMAL_SCALE}"
                )),
            )
        })
}

fn float_precision_type(
    precision: u16,
    scale: Option<u16>,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
    let scale = scale.unwrap_or(0);
    if precision <= 23 && scale <= 7 {
        return Ok(GqlType::Float32);
    }
    if precision <= 52 && scale <= 10 {
        return Ok(GqlType::Float64);
    }
    if precision <= 112 && scale <= 14 {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV25,
            display_name: "128 bit floating point numbers",
            span,
            hint: "this precision/scale request requires FLOAT128, which is outside the selene-db v1.0 claim list",
        });
    }
    Err(ParserError::UnsupportedFeature {
        feature_id: FeatureId::GV26,
        display_name: "256 bit floating point numbers",
        span,
        hint: "this precision/scale request requires FLOAT256 or a floating-point type wider than FLOAT128, which is outside the selene-db v1.0 claim list",
    })
}

fn signed_integer_precision_type(precision: u16, span: SourceSpan) -> Result<GqlType, ParserError> {
    match precision {
        1..=7 => Ok(GqlType::Int8),
        8..=15 => Ok(GqlType::Int16),
        16..=31 => Ok(GqlType::Int32),
        32..=63 => Ok(GqlType::Int64),
        64..=127 => Ok(GqlType::Int128),
        _ => Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV16,
            display_name: "256 bit signed integer numbers",
            span,
            hint: "this precision requires a signed integer wider than INT128, which is outside the selene-db v1.0 claim list",
        }),
    }
}

fn unsigned_integer_precision_type(
    precision: u16,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
    match precision {
        1..=8 => Ok(GqlType::Uint8),
        9..=16 => Ok(GqlType::Uint16),
        17..=32 => Ok(GqlType::Uint32),
        33..=64 => Ok(GqlType::Uint64),
        65..=128 => Ok(GqlType::Uint128),
        _ => Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV15,
            display_name: "256 bit unsigned integer numbers",
            span,
            hint: "this precision requires an unsigned integer wider than UINT128, which is outside the selene-db v1.0 claim list",
        }),
    }
}

fn build_duration_type_name(pair: Pair<'_, Rule>) -> Result<GqlType, ParserError> {
    let source_span = span(&pair);
    let qualifier = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::duration_type)
        .and_then(|duration| {
            duration
                .into_inner()
                .find(|child| child.as_rule() == Rule::temporal_duration_qualifier)
        });
    let Some(qualifier) = qualifier else {
        return Err(ParserError::syntax(
            "DURATION type requires YEAR TO MONTH or DAY TO SECOND qualifier",
            source_span,
            Some("use DURATION (YEAR TO MONTH) or DURATION (DAY TO SECOND)".into()),
        ));
    };
    Ok(match qualifier.as_str().to_ascii_uppercase().as_str() {
        "YEAR TO MONTH" => GqlType::DurationYearToMonth,
        "DAY TO SECOND" => GqlType::DurationDayToSecond,
        _ => unreachable!("grammar restricts temporal_duration_qualifier"),
    })
}

fn build_byte_string_type_name(text: &str, span: SourceSpan) -> Result<GqlType, ParserError> {
    if !text.contains('(') {
        return Ok(GqlType::Bytes);
    }
    let lengths = parse_string_type_lengths(text, span, "byte string")?;
    if keyword_starts_with(text, "BINARY") {
        let [length] = byte_string_single_length(&lengths, span)?;
        return byte_string_type(length, length, ByteStringTypeForm::BinaryFixed, span);
    }
    if keyword_starts_with(text, "VARBINARY") {
        let [max] = byte_string_single_length(&lengths, span)?;
        return byte_string_type(0, max, ByteStringTypeForm::VarbinaryMax, span);
    }
    match lengths.as_slice() {
        [max] => byte_string_type(0, *max, ByteStringTypeForm::BytesMax, span),
        [min, max] => byte_string_type(*min, *max, ByteStringTypeForm::BytesMinMax, span),
        _ => Err(ParserError::syntax(
            "byte string type expects one or two length bounds",
            span,
            None,
        )),
    }
}

fn parse_string_type_lengths(
    text: &str,
    span: SourceSpan,
    kind: &'static str,
) -> Result<Vec<u64>, ParserError> {
    let open = text.find('(').ok_or_else(|| {
        ParserError::syntax(format!("{kind} type is missing length bounds"), span, None)
    })?;
    let close = text.rfind(')').ok_or_else(|| {
        ParserError::syntax(
            format!("{kind} type is missing closing parenthesis"),
            span,
            None,
        )
    })?;
    text[open + 1..close]
        .split(',')
        .map(str::trim)
        .map(|part| {
            part.parse::<u64>().map_err(|_| {
                ParserError::syntax(format!("{kind} length exceeds supported range"), span, None)
            })
        })
        .collect()
}

fn byte_string_single_length(lengths: &[u64], span: SourceSpan) -> Result<[u64; 1], ParserError> {
    string_type_single_length(lengths, span, "byte string")
}

fn string_type_single_length(
    lengths: &[u64],
    span: SourceSpan,
    kind: &'static str,
) -> Result<[u64; 1], ParserError> {
    match lengths {
        [length] => Ok([*length]),
        _ => Err(ParserError::syntax(
            format!("{kind} type expects exactly one length bound"),
            span,
            None,
        )),
    }
}

fn character_string_type(
    min_len: u64,
    max_len: u64,
    form: CharacterStringTypeForm,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
    // Fixed-length coercion pads values up to `min_len`, so an unbounded
    // declared length is an allocation primitive in read-only statements.
    // Checking `max_len` alone is sufficient: `new` rejects min > max.
    if max_len > MAX_CHARACTER_STRING_TYPE_LENGTH {
        return Err(ParserError::syntax(
            "character string length exceeds the implementation-defined maximum",
            span,
            Some(format!(
                "selene-db currently supports declared character string lengths up to {MAX_CHARACTER_STRING_TYPE_LENGTH} characters"
            )),
        ));
    }
    CharacterStringType::new(min_len, max_len, form)
        .map(GqlType::CharacterString)
        .ok_or_else(|| {
            ParserError::syntax(
                "character string length bounds require max > 0 and min <= max",
                span,
                None,
            )
        })
}

fn byte_string_type(
    min_len: u64,
    max_len: u64,
    form: ByteStringTypeForm,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
    // Fixed-length coercion zero-pads values up to `min_len`, so an unbounded
    // declared length is an allocation primitive in read-only statements.
    // Checking `max_len` alone is sufficient: `new` rejects min > max.
    if max_len > MAX_BYTE_STRING_TYPE_LENGTH {
        return Err(ParserError::syntax(
            "byte string length exceeds the implementation-defined maximum",
            span,
            Some(format!(
                "selene-db currently supports declared byte string lengths up to {MAX_BYTE_STRING_TYPE_LENGTH} bytes"
            )),
        ));
    }
    ByteStringType::new(min_len, max_len, form)
        .map(GqlType::ByteString)
        .ok_or_else(|| {
            ParserError::syntax(
                "byte string length bounds require max > 0 and min <= max",
                span,
                None,
            )
        })
}
