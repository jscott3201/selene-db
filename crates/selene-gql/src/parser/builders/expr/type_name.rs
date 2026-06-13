//! GQL type-name builders.

mod strings;

use pest::iterators::Pair;
use selene_core::{DbString, feature_register::FeatureId};

use crate::{
    ast::{
        BindingTableType, DecimalType, GqlType, MAX_DECIMAL_PRECISION, MAX_DECIMAL_SCALE,
        RecordType, SourceSpan,
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
        let mut ty = build_type_name_with_depth(base, depth)?;
        let mut suffix_depth = depth;
        let mut outer_not_null = false;
        for child in children {
            match child.as_rule() {
                Rule::postfix_list_suffix => {
                    suffix_depth += 1;
                    if suffix_depth > MAX_NESTING_DEPTH {
                        return Err(ParserError::NestingLimitExceeded {
                            limit: MAX_NESTING_DEPTH,
                            span: span(&child),
                        });
                    }
                    let child_span = span(&child);
                    let element_not_null = child
                        .into_inner()
                        .any(|part| part.as_rule() == Rule::type_not_null);
                    if element_not_null {
                        ty = apply_not_null(ty, child_span)?;
                    }
                    ty = GqlType::List(Box::new(ty));
                }
                Rule::type_not_null => outer_not_null = true,
                _ => return Err(unexpected_pair(child, "expected type-name suffix")),
            }
        }
        return Ok(if outer_not_null {
            apply_not_null(ty, source_span)?
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
        return strings::build_byte_string_type_name(text, source_span);
    }
    if keyword_starts_with(text, "STRING")
        || keyword_starts_with(text, "CHAR")
        || keyword_starts_with(text, "VARCHAR")
    {
        return strings::build_character_string_type_name(text, source_span);
    }
    if keyword_starts_with(text, "DURATION") {
        return build_duration_type_name(pair);
    }
    if let Some(binding_table_type) = pair
        .clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::binding_table_type)
    {
        return build_binding_table_type_name(binding_table_type, depth);
    }
    if let Some(list_type) = pair
        .clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::angle_list_type)
    {
        let inner = list_type
            .into_inner()
            .find(|child| child.as_rule() == Rule::type_name)
            .ok_or_else(|| {
                ParserError::syntax("list type is missing element type", source_span, None)
            })?;
        return Ok(GqlType::List(Box::new(build_type_name_with_depth(
            inner,
            depth + 1,
        )?)));
    }

    if let Some(record_type) = pair
        .clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::record_type)
    {
        return build_record_type_name(record_type, depth);
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
        (&["ANY", "PROPERTY", "GRAPH"], GqlType::GraphRef),
        (&["PROPERTY", "GRAPH"], GqlType::GraphRef),
        (&["ANY", "GRAPH"], GqlType::GraphRef),
        (&["GRAPH"], GqlType::GraphRef),
        (&["ANY", "NODE"], GqlType::NodeRef),
        (&["NODE"], GqlType::NodeRef),
        (&["ANY", "VERTEX"], GqlType::NodeRef),
        (&["VERTEX"], GqlType::NodeRef),
        (&["ANY", "EDGE"], GqlType::EdgeRef),
        (&["EDGE"], GqlType::EdgeRef),
        (&["ANY", "RELATIONSHIP"], GqlType::EdgeRef),
        (&["RELATIONSHIP"], GqlType::EdgeRef),
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

fn apply_not_null(ty: GqlType, span: SourceSpan) -> Result<GqlType, ParserError> {
    match ty {
        GqlType::Null => Ok(GqlType::Nothing),
        GqlType::Nothing => Err(ParserError::syntax(
            "NOTHING is already the non-null empty type",
            span,
            Some("use NOTHING, or write NULL NOT NULL for the ISO empty type".into()),
        )),
        other => Ok(GqlType::NotNull(Box::new(other))),
    }
}

fn build_record_type_name(pair: Pair<'_, Rule>, depth: u32) -> Result<GqlType, ParserError> {
    // Per ISO/IEC 39075:2024 §18.9, bare `RECORD` and `ANY RECORD` are open
    // record types. A field-types specification, even `{}`, is a closed record
    // type; field syntax itself is §18.10 `<field type>` (`name :: type`).
    for child in pair.into_inner() {
        if child.as_rule() != Rule::field_types_specification {
            continue;
        }
        return Ok(GqlType::Record(RecordType::Closed(
            build_field_types_specification(
                child,
                depth,
                "duplicate record field type name",
                "each closed RECORD type field name must be declared once",
            )?,
        )));
    }
    Ok(GqlType::Record(RecordType::Open))
}

fn build_binding_table_type_name(pair: Pair<'_, Rule>, depth: u32) -> Result<GqlType, ParserError> {
    let source_span = span(&pair);
    let field_spec = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::field_types_specification)
        .ok_or_else(|| {
            ParserError::syntax(
                "binding table type is missing field types specification",
                source_span,
                Some("write TABLE { column :: TYPE, ... }".into()),
            )
        })?;
    Ok(GqlType::TableRef(BindingTableType::Closed(
        build_field_types_specification(
            field_spec,
            depth,
            "duplicate binding table field type name",
            "each TABLE field name must be declared once",
        )?,
    )))
}

fn build_field_types_specification(
    pair: Pair<'_, Rule>,
    depth: u32,
    duplicate_message: &'static str,
    duplicate_hint: &'static str,
) -> Result<Vec<(DbString, GqlType)>, ParserError> {
    let mut fields = Vec::new();
    for field in pair
        .into_inner()
        .filter(|field| field.as_rule() == Rule::record_field_type)
    {
        let field_span = span(&field);
        let mut children = field.into_inner();
        let name_pair = children
            .next()
            .ok_or_else(|| ParserError::syntax("field type is missing name", field_span, None))?;
        let type_pair = children
            .next()
            .ok_or_else(|| ParserError::syntax("field type is missing type", field_span, None))?;
        let name = db_string_pair(name_pair)?;
        if fields
            .iter()
            .any(|(existing_name, _)| existing_name == &name)
        {
            return Err(ParserError::syntax(
                format!("{duplicate_message}: {}", name.as_str()),
                field_span,
                Some(duplicate_hint.into()),
            ));
        }
        fields.push((name, build_type_name_with_depth(type_pair, depth + 1)?));
    }
    Ok(fields)
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
