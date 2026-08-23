//! Session-control builders (ISO/IEC 39075:2024 section 7).

use pest::iterators::Pair;
use selene_core::feature_register::FeatureId;

use crate::{
    ast::{SessionResetTarget, SessionSetGraphTarget, Statement},
    error::ParserError,
};

use super::{Rule, db_string_param, expr, first_child, span, unexpected_pair, unsupported_feature};

/// Build a `session_command` parse tree into a session-control [`Statement`].
pub(super) fn build_session_command(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::session_command);
    let inner = first_child(pair)?;
    match inner.as_rule() {
        Rule::session_set => build_session_set(inner),
        Rule::session_reset => build_session_reset(inner),
        Rule::session_close => Ok(Statement::SessionClose { span: span(&inner) }),
        _ => Err(unexpected_pair(inner, "expected session-control statement")),
    }
}

fn build_session_set(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    let inner = pair
        .into_inner()
        .find(|child| {
            matches!(
                child.as_rule(),
                Rule::session_set_binding_table_parameter
                    | Rule::session_set_graph_parameter
                    | Rule::session_set_graph
                    | Rule::session_set_time_zone
                    | Rule::session_set_value
            )
        })
        .ok_or_else(|| ParserError::syntax("SESSION SET is missing a target", source_span, None))?;
    match inner.as_rule() {
        Rule::session_set_binding_table_parameter => {
            build_session_set_binding_table_parameter(inner)
        }
        Rule::session_set_graph_parameter => build_session_set_graph_parameter(inner),
        Rule::session_set_graph => build_session_set_graph(inner),
        Rule::session_set_time_zone => build_session_set_time_zone(inner),
        Rule::session_set_value => build_session_set_value(inner),
        _ => Err(unexpected_pair(inner, "expected SESSION SET target")),
    }
}

fn build_session_set_binding_table_parameter(
    pair: Pair<'_, Rule>,
) -> Result<Statement, ParserError> {
    let mut param_refs = 0;
    let mut has_subquery = false;
    for child in pair.clone().into_inner() {
        match child.as_rule() {
            Rule::param_ref => param_refs += 1,
            Rule::value_subquery_expr => has_subquery = true,
            _ => {}
        }
    }
    let feature_id = if has_subquery {
        FeatureId::GS10
    } else if param_refs > 1 {
        FeatureId::GS13
    } else {
        FeatureId::GS02
    };
    Err(unsupported_feature(
        &pair,
        feature_id,
        "SESSION SET binding-table parameters are deferred",
    ))
}

fn build_session_set_graph_parameter(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let feature_id = if pair
        .clone()
        .into_inner()
        .any(|child| child.as_rule() == Rule::session_current_graph)
    {
        FeatureId::GS01
    } else {
        FeatureId::GS12
    };
    Err(unsupported_feature(
        &pair,
        feature_id,
        "SESSION SET graph parameters are deferred",
    ))
}

fn build_session_set_graph(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    let target_pair = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::session_current_graph)
        .ok_or_else(|| {
            ParserError::syntax(
                "SESSION SET GRAPH is missing a current graph expression",
                source_span,
                None,
            )
        })?;
    let target = if target_pair
        .as_str()
        .eq_ignore_ascii_case("CURRENT_PROPERTY_GRAPH")
    {
        SessionSetGraphTarget::CurrentPropertyGraph
    } else {
        SessionSetGraphTarget::CurrentGraph
    };
    Ok(Statement::SessionSetGraph {
        target,
        span: source_span,
    })
}

fn build_session_set_time_zone(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    let string_pair = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::string_lit)
        .ok_or_else(|| {
            ParserError::syntax(
                "SESSION SET TIME ZONE is missing a string",
                source_span,
                None,
            )
        })?;
    let (zone, zone_source_kind) = expr::decode_string_text_with_kind(&string_pair)?;
    Ok(Statement::SessionSetTimeZone {
        zone,
        zone_source_kind,
        span: source_span,
    })
}

fn build_session_set_value(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    let mut if_not_exists = false;
    let mut param = None;
    let mut declared_type = None;
    let mut value = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::session_value_kw => {}
            Rule::if_not_exists => if_not_exists = true,
            Rule::param_ref => param = Some(db_string_param(child)?),
            Rule::session_value_declared_type => {
                let type_pair = child
                    .into_inner()
                    .find(|inner| inner.as_rule() == Rule::type_name)
                    .ok_or_else(|| {
                        ParserError::syntax(
                            "SESSION SET VALUE declared type is missing type name",
                            source_span,
                            None,
                        )
                    })?;
                declared_type = Some(expr::build_type_name(type_pair)?);
            }
            // <value specification>: a single literal or parameter reference.
            Rule::session_value_spec => {
                value = Some(expr::build_value_expr(first_child(child)?)?);
            }
            Rule::session_value_subquery_expr => {
                return Err(unsupported_feature(
                    &child,
                    FeatureId::GS11,
                    "SESSION SET VALUE subquery initializers are outside the parser compatibility set",
                ));
            }
            Rule::session_value_simple_expr => {
                return Err(unsupported_feature(
                    &child,
                    FeatureId::GS14,
                    "SESSION SET VALUE simple-expression initializers are outside the parser compatibility set",
                ));
            }
            _ => return Err(unexpected_pair(child, "unexpected SESSION SET VALUE child")),
        }
    }
    let param = param.ok_or_else(|| {
        ParserError::syntax(
            "SESSION SET VALUE is missing a parameter name",
            source_span,
            None,
        )
    })?;
    let value = value.ok_or_else(|| {
        ParserError::syntax(
            "SESSION SET VALUE is missing a value expression",
            source_span,
            None,
        )
    })?;
    Ok(Statement::SessionSetValue {
        param,
        declared_type,
        value: Box::new(value),
        if_not_exists,
        span: source_span,
    })
}

fn build_session_reset(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    // Bare `SESSION RESET` (no arguments) = reset all characteristics
    // (ISO section 7.2 Syntax Rule 2b).
    let Some(args) = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::session_reset_args)
    else {
        return Ok(Statement::SessionReset {
            target: SessionResetTarget::AllCharacteristics,
            span: source_span,
        });
    };
    let inner = first_child(args)?;
    let target = match inner.as_rule() {
        Rule::session_reset_schema => {
            return Err(unsupported_feature(
                &inner,
                FeatureId::GS05,
                "SESSION RESET SCHEMA is not runtime-supported",
            ));
        }
        Rule::session_reset_graph => {
            return Err(unsupported_feature(
                &inner,
                FeatureId::GS06,
                "SESSION RESET GRAPH is not runtime-supported",
            ));
        }
        Rule::session_reset_time_zone => SessionResetTarget::TimeZone,
        Rule::session_reset_all => reset_all_target(&inner),
        Rule::session_reset_parameter => {
            let param_pair = inner
                .into_inner()
                .find(|child| child.as_rule() == Rule::param_ref)
                .ok_or_else(|| {
                    ParserError::syntax(
                        "SESSION RESET PARAMETER is missing a parameter name",
                        source_span,
                        None,
                    )
                })?;
            SessionResetTarget::Parameter(db_string_param(param_pair)?)
        }
        _ => return Err(unexpected_pair(inner, "unexpected SESSION RESET argument")),
    };
    Ok(Statement::SessionReset {
        target,
        span: source_span,
    })
}

/// Distinguish `[ALL] PARAMETERS` from `[ALL] CHARACTERISTICS`.
///
/// Bare `PARAMETERS`/`CHARACTERISTICS` imply `ALL` (ISO section 7.2 Syntax
/// Rule 2a-i), so only the noun matters for target selection.
fn reset_all_target(pair: &Pair<'_, Rule>) -> SessionResetTarget {
    if pair.as_str().to_ascii_uppercase().contains("PARAMETERS") {
        SessionResetTarget::Parameters
    } else {
        SessionResetTarget::AllCharacteristics
    }
}
