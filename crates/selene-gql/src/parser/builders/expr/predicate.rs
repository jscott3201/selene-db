//! Predicate expression builders.

use pest::iterators::Pair;
use selene_core::feature_register::FeatureId;

use crate::{
    ast::{
        BinaryOp, ByteStringType, ByteStringTypeForm, GqlType, IsCheckKind, NormalForm, RecordType,
        SourceSpan, TruthValue, ValueExpr,
    },
    error::ParserError,
    parser::MAX_NESTING_DEPTH,
};

use super::{Rule, build_value_expr, literal};
use crate::parser::builders::{
    db_string_pair, keyword_starts_with, keyword_tokens_eq, not_implemented, pattern, span,
    unexpected_pair,
};

pub(super) fn apply_is_suffix(
    operand: ValueExpr,
    suffix: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    debug_assert_eq!(suffix.as_rule(), Rule::is_suffix);
    let children: Vec<_> = suffix.into_inner().collect();
    // Negation is taken from the parsed `not_kw` token, not by scanning the
    // source text. Substring scans were misleading when an operand contained
    // a quoted identifier with `NOT` in it (e.g. `IS LABELED :"NOT"`).
    let negated = children.iter().any(|child| child.as_rule() == Rule::not_kw);
    dispatch_is_suffix(operand, &children, negated, source_span)
}

fn dispatch_is_suffix(
    operand: ValueExpr,
    children: &[Pair<'_, Rule>],
    negated: bool,
    source_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    if let Some(string_op) = children
        .iter()
        .find(|child| child.as_rule() == Rule::string_match_op)
    {
        return build_string_match(operand, string_op, children, source_span);
    }

    if children.iter().any(|child| child.as_rule() == Rule::in_kw) {
        if let Some(list_pair) = children
            .iter()
            .find(|child| child.as_rule() == Rule::list_lit)
            .cloned()
        {
            return Ok(ValueExpr::InList {
                operand: Box::new(operand),
                list: literal::build_list_items(list_pair)?,
                negated,
                span: source_span,
            });
        }
        let list_expr = find_child(
            children,
            Rule::comparison,
            "IN predicate is missing list expression",
        )?;
        return Ok(ValueExpr::InListExpression {
            operand: Box::new(operand),
            list: Box::new(build_value_expr(list_expr)?),
            negated,
            span: source_span,
        });
    }

    Ok(ValueExpr::IsCheck {
        operand: Box::new(operand),
        kind: build_is_kind(children, source_span)?,
        negated,
        span: source_span,
    })
}

fn build_string_match(
    operand: ValueExpr,
    string_op: &Pair<'_, Rule>,
    children: &[Pair<'_, Rule>],
    source_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    let upper = string_op.as_str().to_ascii_uppercase();
    let op = if upper.starts_with("STARTS") {
        BinaryOp::StartsWith
    } else if upper.starts_with("ENDS") {
        BinaryOp::EndsWith
    } else {
        BinaryOp::Contains
    };
    let rhs_pair = find_child(
        children,
        Rule::comparison,
        "string-match predicate is missing operand",
    )?;
    Ok(ValueExpr::BinaryOp {
        op,
        lhs: Box::new(operand),
        rhs: Box::new(build_value_expr(rhs_pair)?),
        span: source_span,
    })
}

fn build_is_kind(
    children: &[Pair<'_, Rule>],
    source_span: SourceSpan,
) -> Result<IsCheckKind, ParserError> {
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::null_kw)
    {
        return Ok(IsCheckKind::Null);
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::labeled_kw)
    {
        let label_pair = find_child(
            children,
            Rule::label_expr,
            "IS LABELED is missing label expression",
        )?;
        return Ok(IsCheckKind::Labeled(pattern::build_label_expr(label_pair)?));
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::source_of_kw)
    {
        let rhs_pair = find_child(
            children,
            Rule::comparison,
            "IS SOURCE OF is missing expression",
        )?;
        return Ok(IsCheckKind::SourceOf(Box::new(build_value_expr(rhs_pair)?)));
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::destination_of_kw)
    {
        let rhs_pair = find_child(
            children,
            Rule::comparison,
            "IS DESTINATION OF is missing expression",
        )?;
        return Ok(IsCheckKind::DestinationOf(Box::new(build_value_expr(
            rhs_pair,
        )?)));
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::directed_kw)
    {
        return Ok(IsCheckKind::Directed);
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::normalized_kw)
    {
        let form = children
            .iter()
            .find(|child| child.as_rule() == Rule::normal_form)
            .map(|child| match child.as_str().to_ascii_uppercase().as_str() {
                "NFD" => NormalForm::Nfd,
                "NFKC" => NormalForm::Nfkc,
                "NFKD" => NormalForm::Nfkd,
                _ => NormalForm::Nfc,
            })
            .unwrap_or(NormalForm::Nfc);
        return Ok(IsCheckKind::Normalized(form));
    }
    if let Some(truth) = children
        .iter()
        .find(|child| child.as_rule() == Rule::truth_value)
    {
        let value = match truth.as_str().to_ascii_uppercase().as_str() {
            "TRUE" => TruthValue::True,
            "FALSE" => TruthValue::False,
            _ => TruthValue::Unknown,
        };
        return Ok(IsCheckKind::TruthValue(value));
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::typed_kw)
    {
        let type_pair = find_child(children, Rule::type_name, "IS TYPED is missing type")?;
        return Ok(IsCheckKind::Typed(build_type_name(type_pair)?));
    }
    Err(ParserError::syntax(
        "unsupported IS predicate",
        source_span,
        None,
    ))
}

fn find_child<'a>(
    children: &'a [Pair<'a, Rule>],
    rule: Rule,
    missing: &'static str,
) -> Result<Pair<'a, Rule>, ParserError> {
    children
        .iter()
        .find(|child| child.as_rule() == rule)
        .cloned()
        .ok_or_else(|| ParserError::syntax(missing, SourceSpan::default(), None))
}

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
    // temporal spellings (`ZONED DATETIME`, …) keep their multi-token match.
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
        (&["STRING"], GqlType::String),
        (&["VARCHAR"], GqlType::String),
        (&["UUID"], GqlType::Uuid),
        (&["JSON"], GqlType::Json),
        (&["VECTOR"], GqlType::Vector),
        (&["BYTEA"], GqlType::Bytes),
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
            Some("selene-db currently supports integer precision through INT128/UINT128".into()),
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
    let qualifier = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::duration_type)
        .and_then(|duration| {
            duration
                .into_inner()
                .find(|child| child.as_rule() == Rule::temporal_duration_qualifier)
        });
    let Some(qualifier) = qualifier else {
        return Ok(GqlType::Duration);
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
    let lengths = parse_byte_string_lengths(text, span)?;
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

fn parse_byte_string_lengths(text: &str, span: SourceSpan) -> Result<Vec<u64>, ParserError> {
    let open = text.find('(').ok_or_else(|| {
        ParserError::syntax("byte string type is missing length bounds", span, None)
    })?;
    let close = text.rfind(')').ok_or_else(|| {
        ParserError::syntax(
            "byte string type is missing closing parenthesis",
            span,
            None,
        )
    })?;
    text[open + 1..close]
        .split(',')
        .map(str::trim)
        .map(|part| {
            part.parse::<u64>().map_err(|_| {
                ParserError::syntax("byte string length exceeds supported range", span, None)
            })
        })
        .collect()
}

fn byte_string_single_length(lengths: &[u64], span: SourceSpan) -> Result<[u64; 1], ParserError> {
    match lengths {
        [length] => Ok([*length]),
        _ => Err(ParserError::syntax(
            "byte string type expects exactly one length bound",
            span,
            None,
        )),
    }
}

fn byte_string_type(
    min_len: u64,
    max_len: u64,
    form: ByteStringTypeForm,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
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
