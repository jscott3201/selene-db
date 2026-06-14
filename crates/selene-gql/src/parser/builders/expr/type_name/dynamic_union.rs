//! Dynamic union type-name builders.

use pest::iterators::Pair;

use crate::{
    GqlType, SourceSpan,
    error::ParserError,
    parser::{
        MAX_NESTING_DEPTH,
        builders::{span, unexpected_pair},
    },
};

use super::{Rule, build_type_name_with_depth};

pub(super) fn build_open_dynamic_union_type_name(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<GqlType, ParserError> {
    let pair = pair
        .into_inner()
        .find(|child| {
            matches!(
                child.as_rule(),
                Rule::open_dynamic_union_type | Rule::dynamic_property_value_type
            )
        })
        .ok_or_else(|| {
            ParserError::syntax(
                "dynamic union value type is missing open type form",
                source_span,
                None,
            )
        })?;
    let mut rules = pair.into_inner().map(|child| child.as_rule());
    let first = rules.next();
    let second = rules.next();
    let third = rules.next();
    if rules.next().is_some() {
        return Err(ParserError::syntax(
            "unsupported dynamic union value type",
            source_span,
            Some(
                "open dynamic union types are ANY, ANY VALUE, PROPERTY VALUE, and ANY PROPERTY VALUE"
                    .into(),
            ),
        ));
    }

    match (first, second, third) {
        (Some(Rule::any_value_type_kw), None, None)
        | (Some(Rule::any_value_type_kw), Some(Rule::value_type_kw), None) => Ok(GqlType::Any),
        (Some(Rule::property_type_kw), Some(Rule::value_type_kw), None)
        | (
            Some(Rule::any_value_type_kw),
            Some(Rule::property_type_kw),
            Some(Rule::value_type_kw),
        ) => Ok(GqlType::AnyProperty),
        _ => Err(ParserError::syntax(
            "unsupported dynamic union value type",
            source_span,
            Some(
                "open dynamic union types are ANY, ANY VALUE, PROPERTY VALUE, and ANY PROPERTY VALUE"
                    .into(),
            ),
        )),
    }
}

pub(super) fn build_closed_dynamic_union_type_name(
    pair: Pair<'_, Rule>,
    depth: u32,
    source_span: SourceSpan,
) -> Result<GqlType, ParserError> {
    let components = match pair.as_rule() {
        Rule::prefixed_closed_dynamic_union_type => {
            let list = pair
                .into_inner()
                .find(|child| child.as_rule() == Rule::component_type_list)
                .ok_or_else(|| {
                    ParserError::syntax(
                        "closed dynamic union type is missing component type list",
                        source_span,
                        None,
                    )
                })?;
            build_component_type_list(list, depth + 1)?
        }
        _ => return Err(unexpected_pair(pair, "expected closed dynamic union type")),
    };
    validate_closed_dynamic_union_components(components, source_span)
}

pub(super) fn build_closed_dynamic_union_components(
    components: Vec<Pair<'_, Rule>>,
    depth: u32,
    source_span: SourceSpan,
) -> Result<GqlType, ParserError> {
    let components = build_component_type_pairs(components, depth, source_span)?;
    validate_closed_dynamic_union_components(components, source_span)
}

fn build_component_type_list(
    pair: Pair<'_, Rule>,
    depth: u32,
) -> Result<Vec<GqlType>, ParserError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(ParserError::NestingLimitExceeded {
            limit: MAX_NESTING_DEPTH,
            span: span(&pair),
        });
    }
    let mut components = Vec::new();
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::type_name_primary => {
                components.push(build_type_name_with_depth(child, depth)?);
            }
            _ => return Err(unexpected_pair(child, "unexpected component type child")),
        }
    }
    Ok(components)
}

fn build_component_type_pairs(
    pairs: Vec<Pair<'_, Rule>>,
    depth: u32,
    source_span: SourceSpan,
) -> Result<Vec<GqlType>, ParserError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(ParserError::NestingLimitExceeded {
            limit: MAX_NESTING_DEPTH,
            span: source_span,
        });
    }
    let mut components = Vec::with_capacity(pairs.len());
    for pair in pairs {
        match pair.as_rule() {
            Rule::type_name_primary => components.push(build_type_name_with_depth(pair, depth)?),
            _ => return Err(unexpected_pair(pair, "unexpected component type child")),
        }
    }
    Ok(components)
}

fn validate_closed_dynamic_union_components(
    components: Vec<GqlType>,
    source_span: SourceSpan,
) -> Result<GqlType, ParserError> {
    if components.len() < 2 {
        return Err(ParserError::syntax(
            "closed dynamic union type must contain at least two component types",
            source_span,
            Some("ISO GQL disallows a dynamic union type with exactly one component type".into()),
        ));
    }
    let first_not_null = is_known_not_nullable(&components[0]);
    if !components
        .iter()
        .all(|component| is_known_not_nullable(component) == first_not_null)
    {
        return Err(ParserError::syntax(
            "closed dynamic union component types must have uniform nullability",
            source_span,
            Some(
                "write every component as NOT NULL, or leave every component possibly nullable"
                    .into(),
            ),
        ));
    }
    Ok(GqlType::ClosedDynamicUnion(components))
}

fn is_known_not_nullable(ty: &GqlType) -> bool {
    matches!(ty, GqlType::NotNull(_) | GqlType::Nothing)
}
