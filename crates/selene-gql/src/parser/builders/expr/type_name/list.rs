//! List type-name helpers.

use pest::iterators::Pair;

use crate::{
    GqlType,
    error::ParserError,
    parser::builders::{span, unexpected_pair},
};

use super::{Rule, strings};

pub(super) fn build_postfix_list_suffix(
    pair: Pair<'_, Rule>,
) -> Result<(bool, Option<u64>), ParserError> {
    let mut element_not_null = false;
    let mut max_len = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::list_value_type_name_synonym => {}
            Rule::type_not_null => element_not_null = true,
            Rule::list_max_cardinality => max_len = Some(build_list_max_cardinality(child)?),
            _ => {
                return Err(unexpected_pair(
                    child,
                    "unexpected postfix list suffix child",
                ));
            }
        }
    }
    Ok((element_not_null, max_len))
}

pub(super) fn build_list_type(element_type: GqlType, max_len: Option<u64>) -> GqlType {
    match max_len {
        Some(max_len) => GqlType::BoundedList {
            element_type: Box::new(element_type),
            max_len,
        },
        None => GqlType::List(Box::new(element_type)),
    }
}

pub(super) fn build_list_max_cardinality(pair: Pair<'_, Rule>) -> Result<u64, ParserError> {
    let source_span = span(&pair);
    let max_len = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::unsigned_integer)
        .ok_or_else(|| {
            ParserError::syntax("list max cardinality is missing length", source_span, None)
        })
        .and_then(|child| {
            strings::parse_unsigned_length(child.as_str(), span(&child), "list max cardinality")
        })?;
    if max_len == 0 {
        return Err(ParserError::syntax(
            "list max cardinality must be positive",
            source_span,
            Some("use a positive maximum cardinality such as [1]".into()),
        ));
    }
    Ok(max_len)
}
