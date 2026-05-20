//! Data-definition builders.

use pest::iterators::Pair;

use crate::{
    ast::{
        DdlStatement, EdgeEndpointSpec, SourceSpan, TypePropertyConstraint, TypePropertyDef,
        ValidationMode,
    },
    error::ParserError,
    parser::budget::InternerBudget,
};

use super::{Rule, expr, first_child, intern_pair, not_implemented, span, unexpected_pair};

pub(super) fn build_ddl_statement(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<DdlStatement, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::ddl_statement);
    let inner = first_child(pair)?;
    match inner.as_rule() {
        Rule::create_graph => build_create_graph(inner, budget),
        Rule::drop_graph => build_drop_graph(inner, budget),
        Rule::create_node_type => build_create_node_type(inner, budget),
        Rule::create_edge_type => build_create_edge_type(inner, budget),
        Rule::drop_node_type => build_drop_node_type(inner, budget),
        Rule::drop_edge_type => build_drop_edge_type(inner, budget),
        Rule::show_node_types => Ok(DdlStatement::ShowNodeTypes(span(&inner))),
        Rule::show_edge_types => Ok(DdlStatement::ShowEdgeTypes(span(&inner))),
        Rule::show_indexes => Ok(DdlStatement::ShowIndexes(span(&inner))),
        Rule::show_procedures => Ok(DdlStatement::ShowProcedures(span(&inner))),
        Rule::create_trigger | Rule::drop_trigger | Rule::show_triggers => Err(not_implemented(
            &inner,
            "triggers are out of v1.0 scope per spec 03 §7",
        )),
        Rule::create_materialized_view
        | Rule::drop_materialized_view
        | Rule::show_materialized_views => Err(not_implemented(
            &inner,
            "materialized views are not in the v1.0 claim list",
        )),
        Rule::create_procedure | Rule::drop_procedure => Err(not_implemented(
            &inner,
            "procedures are registered through selene-pack, not via DDL",
        )),
        Rule::create_index => Err(not_implemented(
            &inner,
            "named CREATE INDEX DDL requires storage-layer named-index support - landing in a follow-up brief; use `CALL selene.create_index('Label', 'property', 'kind')` today",
        )),
        Rule::drop_index => Err(not_implemented(
            &inner,
            "named DROP INDEX DDL requires storage-layer named-index support - landing in a follow-up brief; use `CALL selene.drop_index('Label', 'property')` today",
        )),
        Rule::create_user
        | Rule::drop_user
        | Rule::create_role
        | Rule::drop_role
        | Rule::grant_role
        | Rule::revoke_role => Err(not_implemented(&inner, "auth is an embedder concern (D1)")),
        _ => Err(unexpected_pair(inner, "expected DDL statement")),
    }
}

fn build_create_graph(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut name = None;
    let mut or_replace = false;
    let mut if_not_exists = false;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::or_replace => or_replace = true,
            Rule::if_not_exists => if_not_exists = true,
            Rule::ident => name = Some(intern_pair(child, budget)?),
            _ => return Err(unexpected_pair(child, "unexpected CREATE GRAPH child")),
        }
    }

    Ok(DdlStatement::CreateGraph {
        name: name.ok_or_else(|| {
            ParserError::syntax("CREATE GRAPH is missing name", source_span, None)
        })?,
        or_replace,
        if_not_exists,
        span: source_span,
    })
}

fn build_drop_graph(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut name = None;
    let mut if_exists = false;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_exists => if_exists = true,
            Rule::ident => name = Some(intern_pair(child, budget)?),
            _ => return Err(unexpected_pair(child, "unexpected DROP GRAPH child")),
        }
    }
    Ok(DdlStatement::DropGraph {
        name: name
            .ok_or_else(|| ParserError::syntax("DROP GRAPH is missing name", source_span, None))?,
        if_exists,
        span: source_span,
    })
}

fn build_create_node_type(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    let mut extends = None;
    let mut or_replace = false;
    let mut if_not_exists = false;
    let mut properties = Vec::new();
    let mut validation_mode = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::or_replace => or_replace = true,
            Rule::if_not_exists => if_not_exists = true,
            Rule::ident if label.is_none() => label = Some(intern_pair(child, budget)?),
            Rule::ident => extends = Some(intern_pair(child, budget)?),
            Rule::type_prop_def_list => properties = build_type_prop_def_list(child, budget)?,
            Rule::validation_mode_clause => validation_mode = Some(build_validation_mode(&child)?),
            _ => return Err(unexpected_pair(child, "unexpected CREATE NODE TYPE child")),
        }
    }

    Ok(DdlStatement::CreateNodeType {
        label: label.ok_or_else(|| {
            ParserError::syntax("CREATE NODE TYPE is missing label", source_span, None)
        })?,
        or_replace,
        if_not_exists,
        extends,
        properties,
        validation_mode,
        span: source_span,
    })
}

fn build_create_edge_type(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    let mut or_replace = false;
    let mut if_not_exists = false;
    let mut endpoints = None;
    let mut properties = Vec::new();
    let mut validation_mode = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::or_replace => or_replace = true,
            Rule::if_not_exists => if_not_exists = true,
            Rule::ident => label = Some(intern_pair(child, budget)?),
            Rule::edge_endpoint_clause => endpoints = Some(build_edge_endpoint(child, budget)?),
            Rule::type_prop_def_list => properties = build_type_prop_def_list(child, budget)?,
            Rule::validation_mode_clause => validation_mode = Some(build_validation_mode(&child)?),
            _ => return Err(unexpected_pair(child, "unexpected CREATE EDGE TYPE child")),
        }
    }

    Ok(DdlStatement::CreateEdgeType {
        label: label.ok_or_else(|| {
            ParserError::syntax("CREATE EDGE TYPE is missing label", source_span, None)
        })?,
        or_replace,
        if_not_exists,
        endpoints,
        properties,
        validation_mode,
        span: source_span,
    })
}

fn build_drop_node_type(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let (label, if_exists) =
        build_drop_type_parts(pair, "DROP NODE TYPE is missing label", budget)?;
    Ok(DdlStatement::DropNodeType {
        label,
        if_exists,
        span: source_span,
    })
}

fn build_drop_edge_type(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let (label, if_exists) =
        build_drop_type_parts(pair, "DROP EDGE TYPE is missing label", budget)?;
    Ok(DdlStatement::DropEdgeType {
        label,
        if_exists,
        span: source_span,
    })
}

fn build_drop_type_parts(
    pair: Pair<'_, Rule>,
    missing: &'static str,
    budget: &mut InternerBudget,
) -> Result<(selene_core::IStr, bool), ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    let mut if_exists = false;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_exists => if_exists = true,
            Rule::ident => label = Some(intern_pair(child, budget)?),
            _ => return Err(unexpected_pair(child, "unexpected DROP TYPE child")),
        }
    }
    Ok((
        label.ok_or_else(|| ParserError::syntax(missing, source_span, None))?,
        if_exists,
    ))
}

fn build_type_prop_def_list(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Vec<TypePropertyDef>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::type_prop_def)
        .map(|child| build_type_prop_def(child, budget))
        .collect()
}

fn build_type_prop_def(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<TypePropertyDef, ParserError> {
    let source_span = span(&pair);
    let mut name = None;
    let mut gql_type = None;
    let mut constraints = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::ident => name = Some(intern_pair(child, budget)?),
            Rule::type_name => gql_type = Some(expr::build_type_name(child)?),
            Rule::type_prop_constraint => {
                constraints.push(build_type_prop_constraint(child, budget)?);
            }
            _ => return Err(unexpected_pair(child, "unexpected type property child")),
        }
    }

    Ok(TypePropertyDef {
        name: name.ok_or_else(|| {
            ParserError::syntax("type property is missing name", source_span, None)
        })?,
        gql_type: gql_type.ok_or_else(|| {
            ParserError::syntax("type property is missing type", source_span, None)
        })?,
        constraints,
        span: source_span,
    })
}

fn build_type_prop_constraint(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<TypePropertyConstraint, ParserError> {
    let source_span = span(&pair);
    let text = pair
        .as_str()
        .to_ascii_uppercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if text == "NOT NULL" {
        return Ok(TypePropertyConstraint::NotNull(source_span));
    }
    if text.starts_with("DEFAULT") {
        let literal = pair
            .into_inner()
            .find(|child| child.as_rule() == Rule::literal)
            .ok_or_else(|| {
                ParserError::syntax("DEFAULT constraint is missing literal", source_span, None)
            })?;
        return Ok(TypePropertyConstraint::Default(
            expr::build_value_expr(literal, budget)?,
            source_span,
        ));
    }
    if text == "IMMUTABLE" {
        return Ok(TypePropertyConstraint::Immutable(source_span));
    }
    if text == "UNIQUE" {
        return Ok(TypePropertyConstraint::Unique(source_span));
    }
    if text == "INDEXED" {
        return Ok(TypePropertyConstraint::Indexed(source_span));
    }
    if text == "SEARCHABLE" {
        return Ok(TypePropertyConstraint::Searchable(source_span));
    }
    if text == "DICTIONARY" {
        return Ok(TypePropertyConstraint::Dictionary(source_span));
    }

    let mut children = pair.into_inner();
    match text.split_whitespace().next() {
        Some("FILL") => {
            let value = child_interned(
                &mut children,
                source_span,
                "FILL is missing strategy",
                budget,
            )?;
            Ok(TypePropertyConstraint::Fill(value, source_span))
        }
        Some("INTERVAL") => {
            let value = children
                .find(|child| child.as_rule() == Rule::string_lit)
                .ok_or_else(|| {
                    ParserError::syntax("INTERVAL is missing duration", source_span, None)
                })
                .and_then(|pair| expr::intern_string_literal(pair, budget))?;
            Ok(TypePropertyConstraint::Interval(value, source_span))
        }
        Some("ENCODING") => {
            let value = child_interned(
                &mut children,
                source_span,
                "ENCODING is missing name",
                budget,
            )?;
            Ok(TypePropertyConstraint::Encoding(value, source_span))
        }
        _ => Err(ParserError::syntax(
            "unknown type property constraint",
            source_span,
            None,
        )),
    }
}

fn build_edge_endpoint(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<EdgeEndpointSpec, ParserError> {
    let source_span = span(&pair);
    let mut lists = pair
        .into_inner()
        .filter(|child| child.as_rule() == Rule::label_list);
    let from_labels = lists
        .next()
        .ok_or_else(|| {
            ParserError::syntax("edge endpoint is missing source labels", source_span, None)
        })
        .and_then(|pair| build_label_list(pair, budget))?;
    let to_labels = lists
        .next()
        .ok_or_else(|| {
            ParserError::syntax("edge endpoint is missing target labels", source_span, None)
        })
        .and_then(|pair| build_label_list(pair, budget))?;

    Ok(EdgeEndpointSpec {
        from_labels,
        to_labels,
        span: source_span,
    })
}

fn build_label_list(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Vec<selene_core::IStr>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::ident)
        .map(|child| intern_pair(child, budget))
        .collect()
}

fn build_validation_mode(pair: &Pair<'_, Rule>) -> Result<ValidationMode, ParserError> {
    match pair.as_str().to_ascii_uppercase().as_str() {
        "STRICT" => Ok(ValidationMode::Strict),
        "WARN" => Ok(ValidationMode::Warn),
        _ => Err(ParserError::syntax(
            "unknown validation mode",
            span(pair),
            None,
        )),
    }
}

fn child_interned(
    children: &mut pest::iterators::Pairs<'_, Rule>,
    source_span: SourceSpan,
    missing: &'static str,
    budget: &mut InternerBudget,
) -> Result<selene_core::IStr, ParserError> {
    children
        .find(|child| child.as_rule() == Rule::ident)
        .ok_or_else(|| ParserError::syntax(missing, source_span, None))
        .and_then(|pair| intern_pair(pair, budget))
}
