//! Graph-pattern pair-to-AST builders.

use pest::iterators::Pair;

use crate::{
    ast::{
        EdgeDirection, EdgePattern, GraphPattern, LabelExpr, MatchClause, MatchMode, NodePattern,
        PathMode, PathSelector, PatternElement, Quantifier, SourceSpan, ValueExpr, Vec2OrMore,
    },
    error::ParserError,
};

use super::{Rule, expr, first_child, intern_pair, keyword_tokens_eq, span, unexpected_pair};

pub(super) fn build_match_clause(pair: Pair<'_, Rule>) -> Result<MatchClause, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::match_stmt);
    let source_span = span(&pair);
    let mut optional = false;
    let mut selector = None;
    let mut match_mode = None;
    let mut path_mode = PathMode::Walk;
    let mut path_mode_explicit = false;
    let mut patterns = Vec::new();
    let mut where_clause = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::optional_modifier => optional = true,
            Rule::path_selector => selector = Some(build_path_selector(&child)?),
            Rule::match_mode => match_mode = Some(build_match_mode(&child)?),
            Rule::path_modifier => {
                path_mode = build_path_mode(&child)?;
                path_mode_explicit = true;
            }
            Rule::graph_pattern_list => patterns = build_graph_pattern_list(child)?,
            Rule::where_clause => where_clause = Some(expr_from_child(child)?),
            _ => return Err(unexpected_pair(child, "unexpected MATCH child")),
        }
    }

    if patterns.is_empty() {
        return Err(ParserError::syntax(
            "MATCH is missing a graph pattern",
            source_span,
            None,
        ));
    }

    Ok(MatchClause {
        optional,
        selector,
        match_mode,
        path_mode,
        path_mode_explicit,
        patterns,
        where_clause,
        span: source_span,
    })
}

fn build_path_selector(pair: &Pair<'_, Rule>) -> Result<PathSelector, ParserError> {
    // Match the selector keyword(s) token-wise (case- and whitespace-
    // insensitive) without allocating a normalized `String`. `ANY SHORTEST` /
    // `ALL SHORTEST` keep their two-token match; check them before the bare
    // single-token `ANY`/`ALL` so the longer spelling wins.
    let text = pair.as_str();
    if keyword_tokens_eq(text, &["ANY", "SHORTEST"]) {
        Ok(PathSelector::AnyShortest)
    } else if keyword_tokens_eq(text, &["ALL", "SHORTEST"]) {
        Ok(PathSelector::AllShortest)
    } else if keyword_tokens_eq(text, &["ANY"]) {
        Ok(PathSelector::Any)
    } else if keyword_tokens_eq(text, &["ALL"]) {
        Ok(PathSelector::All)
    } else {
        Err(ParserError::syntax(
            "unknown path selector",
            span(pair),
            None,
        ))
    }
}

fn build_match_mode(pair: &Pair<'_, Rule>) -> Result<MatchMode, ParserError> {
    let text = pair.as_str().to_ascii_uppercase();
    match text.as_str() {
        "DIFFERENT EDGES" => Ok(MatchMode::DifferentEdges),
        "REPEATABLE ELEMENTS" => Ok(MatchMode::RepeatableElements),
        _ => Err(ParserError::syntax("unknown match mode", span(pair), None)),
    }
}

fn build_path_mode(pair: &Pair<'_, Rule>) -> Result<PathMode, ParserError> {
    let text = pair.as_str().to_ascii_uppercase();
    match text.as_str() {
        "WALK" => Ok(PathMode::Walk),
        "ACYCLIC" => Ok(PathMode::Acyclic),
        "SIMPLE" => Ok(PathMode::Simple),
        "TRAIL" => Ok(PathMode::Trail),
        _ => Err(ParserError::syntax("unknown path mode", span(pair), None)),
    }
}

fn build_graph_pattern_list(pair: Pair<'_, Rule>) -> Result<Vec<GraphPattern>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::graph_pattern)
        .map(|child| build_graph_pattern(child))
        .collect()
}

fn build_graph_pattern(pair: Pair<'_, Rule>) -> Result<GraphPattern, ParserError> {
    let source_span = span(&pair);
    let mut path_binding = None;
    let mut elements = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::path_var_binding => {
                path_binding = Some(intern_pair(first_child(child)?)?);
            }
            Rule::pattern_chain => elements = build_pattern_chain(child)?,
            _ => return Err(unexpected_pair(child, "unexpected graph-pattern child")),
        }
    }

    if elements.is_empty() {
        return Err(ParserError::syntax(
            "graph pattern is empty",
            source_span,
            None,
        ));
    }

    Ok(GraphPattern {
        path_binding,
        elements,
        span: source_span,
    })
}

fn build_pattern_chain(pair: Pair<'_, Rule>) -> Result<Vec<PatternElement>, ParserError> {
    let mut elements = Vec::new();
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::node_pattern => {
                elements.push(PatternElement::Node(build_node_pattern(child)?));
            }
            Rule::edge_pattern => {
                elements.push(PatternElement::Edge(build_edge_pattern(child)?));
            }
            _ => return Err(unexpected_pair(child, "unexpected pattern-chain child")),
        }
    }
    Ok(elements)
}

fn build_node_pattern(pair: Pair<'_, Rule>) -> Result<NodePattern, ParserError> {
    let source_span = span(&pair);
    let mut binding = None;
    let mut label_expr = None;
    let mut properties = Vec::new();
    let mut inline_where = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::node_var => binding = Some(intern_pair(first_child(child)?)?),
            Rule::label_expr => label_expr = Some(build_label_expr(child)?),
            Rule::property_map => properties = build_property_map(child)?,
            Rule::inline_where => inline_where = Some(expr_from_child(child)?),
            _ => return Err(unexpected_pair(child, "unexpected node-pattern child")),
        }
    }

    Ok(NodePattern {
        binding,
        label_expr,
        properties,
        inline_where,
        span: source_span,
    })
}

fn build_edge_pattern(pair: Pair<'_, Rule>) -> Result<EdgePattern, ParserError> {
    let source_span = span(&pair);
    let child = first_child(pair)?;
    let direction = match child.as_rule() {
        Rule::edge_right | Rule::abbrev_right => EdgeDirection::Right,
        Rule::edge_left | Rule::abbrev_left => EdgeDirection::Left,
        Rule::edge_any | Rule::abbrev_any => EdgeDirection::Undirected,
        _ => return Err(unexpected_pair(child, "expected edge pattern")),
    };

    let mut pattern = EdgePattern {
        binding: None,
        direction,
        label_expr: None,
        properties: Vec::new(),
        quantifier: None,
        inline_where: None,
        span: source_span,
    };

    for child in child.into_inner() {
        match child.as_rule() {
            Rule::edge_interior => apply_edge_interior(child, &mut pattern)?,
            Rule::quantifier => assign_quantifier(&mut pattern, child)?,
            _ => return Err(unexpected_pair(child, "unexpected edge-pattern child")),
        }
    }

    Ok(pattern)
}

fn assign_quantifier(pattern: &mut EdgePattern, pair: Pair<'_, Rule>) -> Result<(), ParserError> {
    if pattern.quantifier.is_some() {
        // Quantifiers can appear inside edge_interior or after the closing
        // bracket; the grammar does not forbid both forms in the same edge,
        // so the builder rejects the conflict explicitly. Silently keeping
        // the second occurrence would change traversal bounds without the
        // author noticing.
        return Err(ParserError::syntax(
            "edge pattern carries conflicting quantifiers",
            span(&pair),
            Some("specify the quantifier exactly once".into()),
        ));
    }
    pattern.quantifier = Some(build_quantifier(pair)?);
    Ok(())
}

fn apply_edge_interior(pair: Pair<'_, Rule>, pattern: &mut EdgePattern) -> Result<(), ParserError> {
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::edge_var => pattern.binding = Some(intern_pair(first_child(child)?)?),
            Rule::label_expr => pattern.label_expr = Some(build_label_expr(child)?),
            Rule::property_map => pattern.properties = build_property_map(child)?,
            Rule::quantifier => assign_quantifier(pattern, child)?,
            Rule::inline_where => pattern.inline_where = Some(expr_from_child(child)?),
            _ => return Err(unexpected_pair(child, "unexpected edge-interior child")),
        }
    }
    Ok(())
}

pub(super) fn build_label_expr(pair: Pair<'_, Rule>) -> Result<LabelExpr, ParserError> {
    match pair.as_rule() {
        Rule::label_expr => build_label_expr(first_child(pair)?),
        Rule::label_or => {
            let parts = pair
                .into_inner()
                .filter(|child| child.as_rule() == Rule::label_and)
                .map(|child| build_label_expr(child))
                .collect::<Result<Vec<_>, _>>()?;
            collapse_label_parts(parts, LabelExpr::Disjunction)
        }
        Rule::label_and => {
            let parts = pair
                .into_inner()
                .filter(|child| child.as_rule() == Rule::label_not)
                .map(|child| build_label_expr(child))
                .collect::<Result<Vec<_>, _>>()?;
            collapse_label_parts(parts, LabelExpr::Conjunction)
        }
        Rule::label_not => {
            let negated = pair.as_str().starts_with('!');
            let child = first_child(pair)?;
            let value = build_label_expr(child)?;
            if negated {
                Ok(LabelExpr::Negation(Box::new(value)))
            } else {
                Ok(value)
            }
        }
        Rule::label_atom => build_label_expr(first_child(pair)?),
        Rule::label_wildcard => Ok(LabelExpr::Wildcard),
        Rule::ident => Ok(LabelExpr::Single(intern_pair(pair)?)),
        _ => Err(unexpected_pair(pair, "expected label expression")),
    }
}

fn collapse_label_parts(
    mut parts: Vec<LabelExpr>,
    wrap: fn(Vec2OrMore<LabelExpr>) -> LabelExpr,
) -> Result<LabelExpr, ParserError> {
    match parts.len() {
        0 => Err(ParserError::syntax(
            "label expression is empty",
            SourceSpan::default(),
            None,
        )),
        1 => Ok(parts.remove(0)),
        _ => Ok(wrap(
            Vec2OrMore::try_from_vec(parts).expect("collapse_label_parts only wraps at len >= 2"),
        )),
    }
}

fn build_quantifier(pair: Pair<'_, Rule>) -> Result<Quantifier, ParserError> {
    let source_span = span(&pair);
    let text = pair.as_str();

    if text == "*" {
        return Ok(Quantifier::GraphPattern { min: 0, max: None });
    }
    if text == "+" {
        return Ok(Quantifier::GraphPattern { min: 1, max: None });
    }
    if text == "?" {
        return Ok(Quantifier::Questioned);
    }

    if let Some(range) = text.strip_prefix('*') {
        let (min_text, max_text) = range.split_once("..").ok_or_else(|| {
            ParserError::syntax("invalid variable-length quantifier", source_span, None)
        })?;
        let min = parse_u32(min_text, source_span)?;
        let max = if max_text.is_empty() {
            None
        } else {
            Some(parse_u32(max_text, source_span)?)
        };
        return validate_range(min, max, source_span);
    }

    let body = text
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .ok_or_else(|| ParserError::syntax("invalid quantifier", source_span, None))?;
    if let Some((min_text, max_text)) = body.split_once(',') {
        let min = if min_text.is_empty() {
            0
        } else {
            parse_u32(min_text, source_span)?
        };
        let max = if max_text.is_empty() {
            None
        } else {
            Some(parse_u32(max_text, source_span)?)
        };
        return validate_range(min, max, source_span);
    }

    let exact = parse_u32(body, source_span)?;
    Ok(Quantifier::GraphPattern {
        min: exact,
        max: Some(exact),
    })
}

fn validate_range(
    min: u32,
    max: Option<u32>,
    source_span: SourceSpan,
) -> Result<Quantifier, ParserError> {
    if let Some(max_value) = max
        && max_value < min
    {
        // Reject `*5..2`, `{5,2}`, etc. before they reach planning, where
        // an impossible bound would either be silently empty or violate
        // the planner's monotonicity assumptions.
        return Err(ParserError::syntax(
            "quantifier range has max below min",
            source_span,
            Some("ensure max >= min in `*min..max` or `{min,max}`".into()),
        ));
    }
    Ok(Quantifier::GraphPattern { min, max })
}

fn parse_u32(text: &str, source_span: SourceSpan) -> Result<u32, ParserError> {
    text.parse::<u32>().map_err(|error| {
        ParserError::syntax(
            format!("invalid quantifier bound: {error}"),
            source_span,
            None,
        )
    })
}

pub(super) fn build_property_map(
    pair: Pair<'_, Rule>,
) -> Result<Vec<(selene_core::IStr, ValueExpr)>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::property_pair)
        .map(|property| {
            let property_span = span(&property);
            let mut children = property.into_inner();
            let key = children
                .next()
                .ok_or_else(|| {
                    ParserError::syntax("property pair is missing key", property_span, None)
                })
                .and_then(|pair| intern_pair(pair))?;
            let value = children
                .next()
                .ok_or_else(|| {
                    ParserError::syntax("property pair is missing value", property_span, None)
                })
                .and_then(|pair| expr::build_value_expr(pair))?;
            Ok((key, value))
        })
        .collect()
}

fn expr_from_child(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    pair.into_inner()
        .find(|child| child.as_rule() == Rule::expr)
        .ok_or_else(|| ParserError::syntax("clause is missing expression", source_span, None))
        .and_then(|pair| expr::build_value_expr(pair))
}
