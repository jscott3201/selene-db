//! Database-catalog DDL builders (ISO/IEC 39075:2024 §12.2–§12.5).
//!
//! These builders produce only the forms the database facade executes. Every
//! clause the facade cannot honor is rejected here with the owning feature ID
//! or a not-implemented diagnostic, so an unsupported clause can never be
//! dropped on the floor by a later stage.

use pest::iterators::Pair;
use selene_profile::FeatureId;

use crate::{
    ast::{
        DdlStatement,
        catalog_ref::{CatalogObjectReference, CatalogPathSegment, IdentifierForm},
    },
    error::ParserError,
};

use super::{Rule, db_string_pair, span, unexpected_pair, unsupported_feature};

const PART_TWO_HINT: &str = "closed graph types and CREATE/DROP GRAPH TYPE are delivered by M02-PR04 part 2; write `ANY` for an open graph";

pub(super) fn build_create_schema(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut reference = None;
    let mut if_not_exists = false;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_not_exists => if_not_exists = true,
            Rule::absolute_catalog_path => reference = Some(build_absolute_path(child)?),
            Rule::ddl_create_kw | Rule::ddl_schema_kw => {}
            _ => return Err(unexpected_pair(child, "unexpected CREATE SCHEMA child")),
        }
    }
    Ok(DdlStatement::CreateSchema {
        reference: reference.ok_or_else(|| {
            ParserError::syntax(
                "CREATE SCHEMA is missing its schema path",
                source_span,
                None,
            )
        })?,
        if_not_exists,
        span: source_span,
    })
}

pub(super) fn build_drop_schema(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut reference = None;
    let mut if_exists = false;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_exists => if_exists = true,
            Rule::absolute_catalog_path => reference = Some(build_absolute_path(child)?),
            Rule::ddl_drop_kw | Rule::ddl_schema_kw => {}
            _ => return Err(unexpected_pair(child, "unexpected DROP SCHEMA child")),
        }
    }
    Ok(DdlStatement::DropSchema {
        reference: reference.ok_or_else(|| {
            ParserError::syntax("DROP SCHEMA is missing its schema path", source_span, None)
        })?,
        if_exists,
        span: source_span,
    })
}

/// Build `CREATE GRAPH`, admitting only the `<open graph type>` form.
///
/// Rejections, in the order they are detected: a `<graph source>` cites GG05;
/// `LIKE` cites GG04; an inline `<nested graph type specification>` cites
/// GG03; a `<graph type reference>` is not implemented (GG02, part 2); and a
/// statement with no type clause at all is a syntax error because ISO §12.4
/// makes the clause mandatory.
pub(super) fn build_create_graph(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut reference = None;
    let mut or_replace = false;
    let mut if_not_exists = false;
    let mut open_type = false;
    let mut rejection = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::or_replace => or_replace = true,
            Rule::if_not_exists => if_not_exists = true,
            Rule::graph_reference => reference = Some(build_graph_reference(child)?),
            Rule::create_graph_type_clause => {
                let clause = super::first_child(child)?;
                match clause.as_rule() {
                    Rule::open_graph_type => open_type = true,
                    Rule::graph_type_like_graph => {
                        rejection.get_or_insert(unsupported_feature(&clause, FeatureId::GG04));
                    }
                    Rule::graph_type_inline => {
                        rejection.get_or_insert(unsupported_feature(&clause, FeatureId::GG03));
                    }
                    Rule::graph_type_reference => {
                        rejection.get_or_insert(ParserError::not_implemented(
                            "CREATE GRAPH with an <of graph type> reference binds a closed graph type (Feature GG02)",
                            span(&clause),
                            Some(PART_TWO_HINT),
                        ));
                    }
                    _ => return Err(unexpected_pair(clause, "unexpected graph type clause")),
                }
            }
            // A graph source is the most specific diagnostic even when the
            // type clause is also missing or unsupported.
            Rule::graph_source => rejection = Some(unsupported_feature(&child, FeatureId::GG05)),
            Rule::ddl_create_kw | Rule::ddl_property_kw | Rule::ddl_graph_kw => {}
            _ => return Err(unexpected_pair(child, "unexpected CREATE GRAPH child")),
        }
    }
    if let Some(error) = rejection {
        return Err(error);
    }
    let reference = reference.ok_or_else(|| {
        ParserError::syntax(
            "CREATE GRAPH is missing its graph reference",
            source_span,
            None,
        )
    })?;
    if !open_type {
        return Err(ParserError::syntax(
            "CREATE GRAPH requires an <open graph type> or <of graph type> clause (ISO/IEC 39075:2024 section 12.4)",
            source_span,
            Some("write `CREATE GRAPH name ANY` to create a graph with an open graph type".into()),
        ));
    }
    Ok(DdlStatement::CreateGraph {
        reference,
        or_replace,
        if_not_exists,
        span: source_span,
    })
}

pub(super) fn build_drop_graph(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut reference = None;
    let mut if_exists = false;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_exists => if_exists = true,
            Rule::graph_reference => reference = Some(build_graph_reference(child)?),
            Rule::ddl_drop_kw | Rule::ddl_property_kw | Rule::ddl_graph_kw => {}
            _ => return Err(unexpected_pair(child, "unexpected DROP GRAPH child")),
        }
    }
    Ok(DdlStatement::DropGraph {
        reference: reference.ok_or_else(|| {
            ParserError::syntax(
                "DROP GRAPH is missing its graph reference",
                source_span,
                None,
            )
        })?,
        if_exists,
        span: source_span,
    })
}

/// Reject `CREATE/DROP [PROPERTY] GRAPH TYPE` (§12.6/§12.7) until part 2.
pub(super) fn reject_graph_type_statement(pair: &Pair<'_, Rule>) -> ParserError {
    ParserError::not_implemented(
        "CREATE GRAPH TYPE and DROP GRAPH TYPE (Feature GG02) are not implemented",
        span(pair),
        Some(PART_TWO_HINT),
    )
}

/// Reject a `NEXT`-composed `<linear catalog-modifying statement>` (§12.1).
pub(super) fn reject_catalog_statement_chain(pair: &Pair<'_, Rule>) -> ParserError {
    ParserError::not_implemented(
        "NEXT-composed catalog-modifying statements (ISO/IEC 39075:2024 section 12.1) are not implemented",
        span(pair),
        Some("execute each CREATE/DROP SCHEMA or GRAPH statement as its own request"),
    )
}

fn build_absolute_path(pair: Pair<'_, Rule>) -> Result<CatalogObjectReference, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::absolute_catalog_path);
    let source_span = span(&pair);
    let segments = pair
        .into_inner()
        .filter(|child| child.as_rule() == Rule::ident)
        .map(catalog_segment)
        .collect::<Result<Vec<_>, _>>()?;
    if segments.is_empty() {
        return Err(ParserError::syntax(
            "catalog path has no segments",
            source_span,
            None,
        ));
    }
    Ok(CatalogObjectReference {
        absolute: true,
        segments,
        span: source_span,
    })
}

fn build_graph_reference(pair: Pair<'_, Rule>) -> Result<CatalogObjectReference, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::graph_reference);
    let source_span = span(&pair);
    let inner = super::first_child(pair)?;
    match inner.as_rule() {
        Rule::absolute_catalog_path => build_absolute_path(inner),
        Rule::ident => Ok(CatalogObjectReference {
            absolute: false,
            segments: vec![catalog_segment(inner)?],
            span: source_span,
        }),
        _ => Err(unexpected_pair(inner, "unexpected graph reference child")),
    }
}

/// Decode one `ident` while keeping its lexical form.
///
/// [`db_string_pair`] erases the form, which is fine for labels and property
/// keys but not for catalog paths: the catalog validates regular and delimited
/// names under different rules, and a delimited segment may legitimately spell
/// `/`, spaces, or backticks.
fn catalog_segment(pair: Pair<'_, Rule>) -> Result<CatalogPathSegment, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::ident);
    let form = if pair.as_str().starts_with(['"', '`']) {
        IdentifierForm::Delimited
    } else {
        IdentifierForm::Regular
    };
    Ok(CatalogPathSegment {
        name: db_string_pair(pair)?,
        form,
    })
}
