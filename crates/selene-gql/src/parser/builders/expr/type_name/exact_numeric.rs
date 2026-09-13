//! Exact numeric type-name helpers.

use pest::iterators::Pair;

use crate::{GqlType, error::ParserError, parser::builders::span};

use super::Rule;

pub(super) fn build_keyword_type_name(
    pair: &Pair<'_, Rule>,
) -> Result<Option<GqlType>, ParserError> {
    let rules = pair
        .clone()
        .into_inner()
        .map(|child| child.as_rule())
        .collect::<Vec<_>>();

    if matches!(
        rules.as_slice(),
        [Rule::uint_256_kw] | [Rule::unsigned_kw, Rule::integer_256_kw]
    ) {
        return Err(ParserError::UnsupportedFeature {
            feature_id: selene_profile::FeatureId::GV15,
            display_name: "256 bit unsigned integer numbers",
            span: span(pair),
            hint: "UINT256 is runtime-unsupported",
        });
    }
    if matches!(
        rules.as_slice(),
        [Rule::int_256_kw] | [Rule::integer_256_kw] | [Rule::signed_kw, Rule::integer_256_kw]
    ) {
        return Err(ParserError::UnsupportedFeature {
            feature_id: selene_profile::FeatureId::GV16,
            display_name: "256 bit signed integer numbers",
            span: span(pair),
            hint: "INT256 is runtime-unsupported",
        });
    }

    let ty = match rules.as_slice() {
        [Rule::boolean_kw] | [Rule::bool_kw] => GqlType::Boolean,
        [Rule::signed_kw, Rule::small_kw, Rule::integer_kw] => GqlType::SmallInt,
        [Rule::signed_kw, Rule::big_kw, Rule::integer_kw] => GqlType::BigInt,
        [Rule::signed_kw, Rule::integer_8_kw] => GqlType::Int8,
        [Rule::signed_kw, Rule::integer_16_kw] => GqlType::Int16,
        [Rule::signed_kw, Rule::integer_32_kw] => GqlType::Int32,
        [Rule::signed_kw, Rule::integer_64_kw] => GqlType::Int64,
        [Rule::signed_kw, Rule::integer_128_kw] => GqlType::Int128,
        [Rule::signed_kw, Rule::integer_kw] => GqlType::Integer,
        [Rule::unsigned_kw, Rule::small_kw, Rule::integer_kw] => GqlType::USmallInt,
        [Rule::unsigned_kw, Rule::big_kw, Rule::integer_kw] => GqlType::UBigInt,
        [Rule::unsigned_kw, Rule::integer_8_kw] => GqlType::Uint8,
        [Rule::unsigned_kw, Rule::integer_16_kw] => GqlType::Uint16,
        [Rule::unsigned_kw, Rule::integer_32_kw] => GqlType::Uint32,
        [Rule::unsigned_kw, Rule::integer_64_kw] => GqlType::Uint64,
        [Rule::unsigned_kw, Rule::integer_128_kw] => GqlType::Uint128,
        [Rule::unsigned_kw, Rule::integer_kw] => GqlType::Uint,
        [Rule::big_kw, Rule::integer_kw] | [Rule::bigint_kw] => GqlType::BigInt,
        [Rule::small_kw, Rule::integer_kw] | [Rule::smallint_kw] => GqlType::SmallInt,
        [Rule::integer_8_kw] | [Rule::int_8_kw] => GqlType::Int8,
        [Rule::integer_16_kw] | [Rule::int_16_kw] => GqlType::Int16,
        [Rule::integer_32_kw] | [Rule::int_32_kw] => GqlType::Int32,
        [Rule::integer_64_kw] | [Rule::int_64_kw] => GqlType::Int64,
        [Rule::integer_128_kw] | [Rule::int_128_kw] => GqlType::Int128,
        [Rule::integer_kw] | [Rule::int_kw] => GqlType::Integer,
        [Rule::uint_kw] => GqlType::Uint,
        [Rule::uint_8_kw] => GqlType::Uint8,
        [Rule::uint_16_kw] => GqlType::Uint16,
        [Rule::uint_32_kw] => GqlType::Uint32,
        [Rule::uint_64_kw] => GqlType::Uint64,
        [Rule::uint_128_kw] => GqlType::Uint128,
        [Rule::usmallint_kw] => GqlType::USmallInt,
        [Rule::ubigint_kw] => GqlType::UBigInt,
        _ => return Ok(None),
    };
    Ok(Some(ty))
}
