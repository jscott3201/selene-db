//! Temporal type-name helpers.

use pest::iterators::Pair;

use crate::{GqlType, error::ParserError, parser::builders::span};

use super::Rule;

pub(super) fn build_keyword_type_name(pair: &Pair<'_, Rule>) -> Option<GqlType> {
    let mut rules = pair.clone().into_inner().map(|child| child.as_rule());
    let first = rules.next()?;
    let second = rules.next();
    let third = rules.next();
    let fourth = rules.next();
    if rules.next().is_some() {
        return None;
    }

    match (first, second, third, fourth) {
        (Rule::timestamp_kw, Some(Rule::with_kw), Some(Rule::time_kw), Some(Rule::zone_kw)) => {
            Some(GqlType::ZonedDateTime)
        }
        (Rule::timestamp_kw, Some(Rule::without_kw), Some(Rule::time_kw), Some(Rule::zone_kw)) => {
            Some(GqlType::LocalDateTime)
        }
        (Rule::timestamp_kw, None, None, None) => Some(GqlType::LocalDateTime),
        (Rule::time_kw, Some(Rule::with_kw), Some(Rule::time_kw), Some(Rule::zone_kw)) => {
            Some(GqlType::ZonedTime)
        }
        (Rule::time_kw, Some(Rule::without_kw), Some(Rule::time_kw), Some(Rule::zone_kw)) => {
            Some(GqlType::LocalTime)
        }
        (Rule::zoned_kw, Some(Rule::datetime_kw), None, None) => Some(GqlType::ZonedDateTime),
        (Rule::local_kw, Some(Rule::datetime_kw), None, None) => Some(GqlType::LocalDateTime),
        (Rule::zoned_kw, Some(Rule::time_kw), None, None) => Some(GqlType::ZonedTime),
        (Rule::local_kw, Some(Rule::time_kw), None, None) => Some(GqlType::LocalTime),
        (Rule::date_kw, None, None, None) => Some(GqlType::Date),
        _ => None,
    }
}

pub(super) fn build_duration_type_name(pair: Pair<'_, Rule>) -> Result<GqlType, ParserError> {
    let source_span = span(&pair);
    let qualifier_rule = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::duration_type)
        .and_then(|duration| {
            duration
                .into_inner()
                .find(|child| child.as_rule() == Rule::temporal_duration_qualifier)
        })
        .and_then(|qualifier| qualifier.into_inner().next().map(|child| child.as_rule()));

    match qualifier_rule {
        Some(Rule::year_to_month_qualifier) => Ok(GqlType::DurationYearToMonth),
        Some(Rule::day_to_second_qualifier) => Ok(GqlType::DurationDayToSecond),
        _ => Err(ParserError::syntax(
            "DURATION type requires YEAR TO MONTH or DAY TO SECOND qualifier",
            source_span,
            Some("use DURATION (YEAR TO MONTH) or DURATION (DAY TO SECOND)".into()),
        )),
    }
}
