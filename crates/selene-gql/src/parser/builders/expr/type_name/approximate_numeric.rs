//! Approximate numeric type-name helpers.

use pest::iterators::Pair;
use selene_core::feature_register::FeatureId;

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

    match rules.as_slice() {
        [Rule::float_16_kw] => Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV20,
            display_name: "16 bit floating point numbers",
            span: span(pair),
            hint: "FLOAT16 is runtime-unsupported; use FLOAT32 or FLOAT64",
        }),
        [Rule::float_128_kw] => Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV25,
            display_name: "128 bit floating point numbers",
            span: span(pair),
            hint: "FLOAT128 is runtime-unsupported",
        }),
        [Rule::float_256_kw] => Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV26,
            display_name: "256 bit floating point numbers",
            span: span(pair),
            hint: "FLOAT256 is runtime-unsupported",
        }),
        [Rule::float_kw] => Ok(Some(GqlType::Float)),
        [Rule::float_32_kw] => Ok(Some(GqlType::Float32)),
        [Rule::float_64_kw] => Ok(Some(GqlType::Float64)),
        [Rule::real_kw] => Ok(Some(GqlType::Real)),
        [Rule::double_kw] | [Rule::double_kw, Rule::precision_kw] => Ok(Some(GqlType::Double)),
        _ => Ok(None),
    }
}
