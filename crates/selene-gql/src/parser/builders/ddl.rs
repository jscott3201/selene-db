//! Data-definition builders.

use pest::iterators::Pair;

use crate::{
    ast::{
        DdlStatement, DropBehavior, EdgeEndpointSpec, TypePropertyConstraint, TypePropertyDef,
        ValidationMode,
    },
    error::ParserError,
};

use super::{
    Rule, expr, first_child, intern_pair, keyword_starts_with, keyword_tokens_eq, span,
    unexpected_pair,
};

pub(super) fn build_ddl_statement(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::ddl_statement);
    let inner = first_child(pair)?;
    match inner.as_rule() {
        Rule::create_graph => build_create_graph(inner),
        Rule::drop_graph => build_drop_graph(inner),
        Rule::create_node_type => build_create_node_type(inner),
        Rule::create_edge_type => build_create_edge_type(inner),
        Rule::drop_node_type => build_drop_node_type(inner),
        Rule::drop_edge_type => build_drop_edge_type(inner),
        Rule::truncate_node_type => build_truncate_node_type(inner),
        Rule::truncate_edge_type => build_truncate_edge_type(inner),
        Rule::show_node_types => Ok(DdlStatement::ShowNodeTypes(span(&inner))),
        Rule::show_edge_types => Ok(DdlStatement::ShowEdgeTypes(span(&inner))),
        Rule::show_indexes => Ok(DdlStatement::ShowIndexes(span(&inner))),
        Rule::show_procedures => Ok(DdlStatement::ShowProcedures(span(&inner))),
        Rule::create_index => build_create_index(inner),
        Rule::drop_index => build_drop_index(inner),
        _ => Err(unexpected_pair(inner, "expected DDL statement")),
    }
}

fn build_create_graph(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut name = None;
    let mut or_replace = false;
    let mut if_not_exists = false;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::or_replace => or_replace = true,
            Rule::if_not_exists => if_not_exists = true,
            Rule::ident => name = Some(intern_pair(child)?),
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

fn build_drop_graph(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut name = None;
    let mut if_exists = false;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_exists => if_exists = true,
            Rule::ident => name = Some(intern_pair(child)?),
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

fn build_create_index(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut if_not_exists = false;
    let mut idents = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_not_exists => if_not_exists = true,
            Rule::ident => idents.push(intern_pair(child)?),
            _ => return Err(unexpected_pair(child, "unexpected CREATE INDEX child")),
        }
    }

    let mut idents = idents.into_iter();
    let name = idents.next().ok_or_else(|| {
        ParserError::syntax("CREATE INDEX is missing index name", source_span, None)
    })?;
    let label = idents.next().ok_or_else(|| {
        ParserError::syntax("CREATE INDEX is missing target label", source_span, None)
    })?;
    let properties = idents.collect::<Vec<_>>();
    if properties.is_empty() {
        return Err(ParserError::syntax(
            "CREATE INDEX is missing property name",
            source_span,
            None,
        ));
    }

    Ok(DdlStatement::CreateIndex {
        name,
        label,
        properties,
        if_not_exists,
        span: source_span,
    })
}

fn build_drop_index(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut name = None;
    let mut if_exists = false;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_exists => if_exists = true,
            Rule::ident => name = Some(intern_pair(child)?),
            _ => return Err(unexpected_pair(child, "unexpected DROP INDEX child")),
        }
    }

    Ok(DdlStatement::DropIndex {
        name: name.ok_or_else(|| {
            ParserError::syntax("DROP INDEX is missing index name", source_span, None)
        })?,
        if_exists,
        span: source_span,
    })
}

fn build_create_node_type(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
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
            Rule::ident if label.is_none() => label = Some(intern_pair(child)?),
            Rule::ident => extends = Some(intern_pair(child)?),
            Rule::type_prop_def_list => properties = build_type_prop_def_list(child)?,
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

fn build_create_edge_type(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    let mut extends = None;
    let mut or_replace = false;
    let mut if_not_exists = false;
    let mut endpoints = None;
    let mut properties = Vec::new();
    let mut validation_mode = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::or_replace => or_replace = true,
            Rule::if_not_exists => if_not_exists = true,
            Rule::ident if label.is_none() => label = Some(intern_pair(child)?),
            Rule::ident => extends = Some(intern_pair(child)?),
            Rule::edge_endpoint_clause => endpoints = Some(build_edge_endpoint(child)?),
            Rule::type_prop_def_list => properties = build_type_prop_def_list(child)?,
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
        extends,
        endpoints,
        properties,
        validation_mode,
        span: source_span,
    })
}

fn build_drop_node_type(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let (label, if_exists, behavior) =
        build_drop_type_parts(pair, "DROP NODE TYPE is missing label")?;
    Ok(DdlStatement::DropNodeType {
        label,
        if_exists,
        behavior,
        span: source_span,
    })
}

fn build_drop_edge_type(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let (label, if_exists, behavior) =
        build_drop_type_parts(pair, "DROP EDGE TYPE is missing label")?;
    Ok(DdlStatement::DropEdgeType {
        label,
        if_exists,
        behavior,
        span: source_span,
    })
}

fn build_truncate_node_type(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let label = build_truncate_label(pair, "TRUNCATE NODE TYPE is missing label")?;
    Ok(DdlStatement::TruncateNodeType {
        label,
        span: source_span,
    })
}

fn build_truncate_edge_type(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let label = build_truncate_label(pair, "TRUNCATE EDGE TYPE is missing label")?;
    Ok(DdlStatement::TruncateEdgeType {
        label,
        span: source_span,
    })
}

fn build_truncate_label(
    pair: Pair<'_, Rule>,
    missing: &'static str,
) -> Result<selene_core::IStr, ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::ident => label = Some(intern_pair(child)?),
            _ => return Err(unexpected_pair(child, "unexpected TRUNCATE TYPE child")),
        }
    }
    label.ok_or_else(|| ParserError::syntax(missing, source_span, None))
}

fn build_drop_type_parts(
    pair: Pair<'_, Rule>,
    missing: &'static str,
) -> Result<(selene_core::IStr, bool, DropBehavior), ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    let mut if_exists = false;
    // Default behavior when the optional `RESTRICT | CASCADE` tail is absent.
    let mut behavior = DropBehavior::Restrict;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_exists => if_exists = true,
            Rule::ident => label = Some(intern_pair(child)?),
            Rule::drop_behavior => behavior = build_drop_behavior(&child)?,
            _ => return Err(unexpected_pair(child, "unexpected DROP TYPE child")),
        }
    }
    Ok((
        label.ok_or_else(|| ParserError::syntax(missing, source_span, None))?,
        if_exists,
        behavior,
    ))
}

fn build_drop_behavior(pair: &Pair<'_, Rule>) -> Result<DropBehavior, ParserError> {
    match pair.as_str().to_ascii_uppercase().as_str() {
        "RESTRICT" => Ok(DropBehavior::Restrict),
        "CASCADE" => Ok(DropBehavior::Cascade),
        _ => Err(ParserError::syntax(
            "unknown drop behavior",
            span(pair),
            None,
        )),
    }
}

fn build_type_prop_def_list(pair: Pair<'_, Rule>) -> Result<Vec<TypePropertyDef>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::type_prop_def)
        .map(|child| build_type_prop_def(child))
        .collect()
}

fn build_type_prop_def(pair: Pair<'_, Rule>) -> Result<TypePropertyDef, ParserError> {
    let source_span = span(&pair);
    let mut name = None;
    let mut gql_type = None;
    let mut constraints = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::ident => name = Some(intern_pair(child)?),
            Rule::type_name => gql_type = Some(expr::build_type_name(child)?),
            Rule::type_prop_constraint => {
                constraints.push(build_type_prop_constraint(child)?);
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

fn build_type_prop_constraint(pair: Pair<'_, Rule>) -> Result<TypePropertyConstraint, ParserError> {
    let source_span = span(&pair);
    // Match the constraint keyword(s) token-wise (case- and whitespace-
    // insensitive) without allocating a normalized `String`. `NOT NULL` keeps
    // its two-token match; `DEFAULT`/`INDEXED` are leading-keyword prefixes
    // (the literal / index name are read from the pest children).
    let text = pair.as_str();

    if keyword_tokens_eq(text, &["NOT", "NULL"]) {
        return Ok(TypePropertyConstraint::NotNull(source_span));
    }
    if keyword_starts_with(text, "DEFAULT") {
        let literal = pair
            .into_inner()
            .find(|child| child.as_rule() == Rule::literal)
            .ok_or_else(|| {
                ParserError::syntax("DEFAULT constraint is missing literal", source_span, None)
            })?;
        return Ok(TypePropertyConstraint::Default(
            expr::build_value_expr(literal)?,
            source_span,
        ));
    }
    if keyword_tokens_eq(text, &["IMMUTABLE"]) {
        return Ok(TypePropertyConstraint::Immutable(source_span));
    }
    if keyword_tokens_eq(text, &["UNIQUE"]) {
        return Ok(TypePropertyConstraint::Unique(source_span));
    }
    if keyword_starts_with(text, "INDEXED") {
        let name = pair
            .into_inner()
            .find(|child| child.as_rule() == Rule::ident)
            .map(|child| intern_pair(child))
            .transpose()?;
        return Ok(TypePropertyConstraint::Indexed {
            name,
            span: source_span,
        });
    }

    Err(ParserError::syntax(
        "unknown type property constraint",
        source_span,
        None,
    ))
}

fn build_edge_endpoint(pair: Pair<'_, Rule>) -> Result<EdgeEndpointSpec, ParserError> {
    let source_span = span(&pair);
    let mut lists = pair
        .into_inner()
        .filter(|child| child.as_rule() == Rule::label_list);
    let from_labels = lists
        .next()
        .ok_or_else(|| {
            ParserError::syntax("edge endpoint is missing source labels", source_span, None)
        })
        .and_then(|pair| build_label_list(pair))?;
    let to_labels = lists
        .next()
        .ok_or_else(|| {
            ParserError::syntax("edge endpoint is missing target labels", source_span, None)
        })
        .and_then(|pair| build_label_list(pair))?;

    Ok(EdgeEndpointSpec {
        from_labels,
        to_labels,
        span: source_span,
    })
}

fn build_label_list(pair: Pair<'_, Rule>) -> Result<Vec<selene_core::IStr>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::ident)
        .map(|child| intern_pair(child))
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
