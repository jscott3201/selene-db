//! Data-definition builders.

use pest::iterators::Pair;

use selene_core::DbString;

use crate::{
    ast::{
        DdlStatement, DropBehavior, EdgeEndpointSpec, KeyLabelSet, TypePropertyConstraint,
        TypePropertyDef, ValidationMode,
    },
    error::ParserError,
};

use super::{
    Rule, catalog_ddl, db_string_pair, expr, first_child, keyword_starts_with, keyword_tokens_eq,
    span, unexpected_pair,
};

pub(super) fn build_ddl_statement(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::ddl_statement);
    let inner = first_child(pair)?;
    match inner.as_rule() {
        Rule::create_schema => catalog_ddl::build_create_schema(inner),
        Rule::drop_schema => catalog_ddl::build_drop_schema(inner),
        Rule::create_graph => catalog_ddl::build_create_graph(inner),
        Rule::drop_graph => catalog_ddl::build_drop_graph(inner),
        Rule::create_graph_type => catalog_ddl::build_create_graph_type(inner),
        Rule::drop_graph_type => catalog_ddl::build_drop_graph_type(inner),
        Rule::create_node_type => build_create_node_type(inner),
        Rule::alter_node_type => build_alter_node_type(inner),
        Rule::create_edge_type => build_create_edge_type(inner),
        Rule::alter_edge_type => build_alter_edge_type(inner),
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

fn is_ddl_keyword_token(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::ddl_create_kw
            | Rule::ddl_alter_kw
            | Rule::ddl_drop_kw
            | Rule::ddl_truncate_kw
            | Rule::ddl_node_kw
            | Rule::ddl_edge_kw
            | Rule::ddl_type_kw
            | Rule::ddl_graph_kw
            | Rule::ddl_index_kw
            | Rule::ddl_extends_kw
            | Rule::ddl_on_kw
    )
}

fn build_create_index(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut if_not_exists = false;
    let mut name = None;
    let mut label = None;
    let mut properties = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_not_exists => if_not_exists = true,
            Rule::ident if name.is_none() => name = Some(db_string_pair(child)?),
            Rule::ident if label.is_none() => label = Some(db_string_pair(child)?),
            Rule::prop_ident => properties.push(db_string_pair(child)?),
            rule if is_ddl_keyword_token(rule) => {}
            _ => return Err(unexpected_pair(child, "unexpected CREATE INDEX child")),
        }
    }

    if properties.is_empty() {
        return Err(ParserError::syntax(
            "CREATE INDEX is missing property name",
            source_span,
            None,
        ));
    }

    Ok(DdlStatement::CreateIndex {
        name: name.ok_or_else(|| {
            ParserError::syntax("CREATE INDEX is missing index name", source_span, None)
        })?,
        label: label.ok_or_else(|| {
            ParserError::syntax("CREATE INDEX is missing target label", source_span, None)
        })?,
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
            Rule::ident => name = Some(db_string_pair(child)?),
            rule if is_ddl_keyword_token(rule) => {}
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
    let mut descriptor = None;
    let mut extends = None;
    let mut or_replace = false;
    let mut if_not_exists = false;
    let mut properties = Vec::new();
    let mut validation_mode = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::or_replace => or_replace = true,
            Rule::if_not_exists => if_not_exists = true,
            Rule::node_type_descriptor => descriptor = Some(build_type_descriptor(child)?),
            // The `EXTENDS :ident` parent is the only bare `ident` child once the
            // element-type descriptor is its own rule.
            Rule::ident => extends = Some(db_string_pair(child)?),
            Rule::type_prop_def_list => properties = build_type_prop_def_list(child)?,
            Rule::validation_mode_clause => validation_mode = Some(build_validation_mode(&child)?),
            rule if is_ddl_keyword_token(rule) => {}
            _ => return Err(unexpected_pair(child, "unexpected CREATE NODE TYPE child")),
        }
    }

    let (label, key_label_set) = descriptor.ok_or_else(|| {
        ParserError::syntax("CREATE NODE TYPE is missing label", source_span, None)
    })?;

    Ok(DdlStatement::CreateNodeType {
        label,
        key_label_set,
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
    let mut descriptor = None;
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
            Rule::edge_type_descriptor => descriptor = Some(build_type_descriptor(child)?),
            Rule::ident => extends = Some(db_string_pair(child)?),
            Rule::edge_endpoint_clause => endpoints = Some(build_edge_endpoint(child)?),
            Rule::type_prop_def_list => properties = build_type_prop_def_list(child)?,
            Rule::validation_mode_clause => validation_mode = Some(build_validation_mode(&child)?),
            rule if is_ddl_keyword_token(rule) => {}
            _ => return Err(unexpected_pair(child, "unexpected CREATE EDGE TYPE child")),
        }
    }

    let (label, key_label_set) = descriptor.ok_or_else(|| {
        ParserError::syntax("CREATE EDGE TYPE is missing label", source_span, None)
    })?;

    Ok(DdlStatement::CreateEdgeType {
        label,
        key_label_set,
        or_replace,
        if_not_exists,
        extends,
        endpoints,
        properties,
        validation_mode,
        span: source_span,
    })
}

fn build_alter_node_type(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    let mut properties = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::ident => label = Some(db_string_pair(child)?),
            Rule::type_prop_def_list => properties = build_type_prop_def_list(child)?,
            rule if is_ddl_keyword_token(rule) => {}
            _ => return Err(unexpected_pair(child, "unexpected ALTER NODE TYPE child")),
        }
    }
    if properties.is_empty() {
        return Err(ParserError::syntax(
            "ALTER NODE TYPE must declare at least one property",
            source_span,
            None,
        ));
    }
    Ok(DdlStatement::AlterNodeType {
        label: label.ok_or_else(|| {
            ParserError::syntax("ALTER NODE TYPE is missing label", source_span, None)
        })?,
        properties,
        span: source_span,
    })
}

fn build_alter_edge_type(pair: Pair<'_, Rule>) -> Result<DdlStatement, ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    let mut endpoints = None;
    let mut properties = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::ident => label = Some(db_string_pair(child)?),
            Rule::edge_endpoint_clause => endpoints = Some(build_edge_endpoint(child)?),
            Rule::type_prop_def_list => properties = build_type_prop_def_list(child)?,
            rule if is_ddl_keyword_token(rule) => {}
            _ => return Err(unexpected_pair(child, "unexpected ALTER EDGE TYPE child")),
        }
    }
    if endpoints.is_none() && properties.is_empty() {
        return Err(ParserError::syntax(
            "ALTER EDGE TYPE must declare endpoints or properties",
            source_span,
            None,
        ));
    }
    Ok(DdlStatement::AlterEdgeType {
        label: label.ok_or_else(|| {
            ParserError::syntax("ALTER EDGE TYPE is missing label", source_span, None)
        })?,
        endpoints,
        properties,
        span: source_span,
    })
}

/// Build the element-type descriptor: either a bare `<...type name>` (`:Name`)
/// or an explicit `<...type key label set>` (Feature GG21). Returns the derived
/// type-name label plus the optional explicit key label set.
///
/// The `node_type_descriptor`/`edge_type_descriptor` rule wraps either a single
/// `ident` (the bare-name GG20 path) or a `*_type_key_label_set` rule (the GG21
/// path). For GG21 the preferred type-name label is the first key-label-set
/// phrase label (ISO §18.2 SR5a — the key label set is the phrase's labels). A
/// bare `<implies>` with no phrase has no label; that empty key label set is
/// left for the plan-time IL003 minimum-cardinality reject (42012/42014), so an
/// empty placeholder name is constructed to keep the AST total — it never reaches
/// the catalog because the plan rejects first.
fn build_type_descriptor(
    pair: Pair<'_, Rule>,
) -> Result<(DbString, Option<KeyLabelSet>), ParserError> {
    let descriptor_span = span(&pair);
    let inner = first_child(pair)?;
    match inner.as_rule() {
        Rule::ident => Ok((db_string_pair(inner)?, None)),
        Rule::node_type_key_label_set | Rule::edge_type_key_label_set => {
            let key_label_set = build_key_label_set(inner)?;
            let label = match key_label_set.labels.first() {
                Some(label) => label.clone(),
                None => selene_core::db_string("").map_err(|_err| {
                    ParserError::syntax(
                        "could not construct database string for empty key-label-set placeholder name",
                        descriptor_span,
                        None,
                    )
                })?,
            };
            Ok((label, Some(key_label_set)))
        }
        _ => Err(unexpected_pair(inner, "unexpected element type descriptor")),
    }
}

/// Build a [`KeyLabelSet`] from a `node_type_key_label_set`/
/// `edge_type_key_label_set` pair (`[ <label set phrase> ] <implies> [ <implied
/// label set> ]`).
fn build_key_label_set(pair: Pair<'_, Rule>) -> Result<KeyLabelSet, ParserError> {
    let key_label_span = span(&pair);
    let mut labels = Vec::new();
    let mut implied_labels = Vec::new();
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::key_label_set_phrase => labels = build_key_label_phrase(child)?,
            Rule::implies => {}
            Rule::key_label_implied_labels => implied_labels = build_key_label_phrase(child)?,
            _ => return Err(unexpected_pair(child, "unexpected key label set child")),
        }
    }
    Ok(KeyLabelSet {
        labels,
        implied_labels,
        span: key_label_span,
    })
}

/// Collect the `:Label (& :Label)*` identifiers of a key-label-set phrase.
fn build_key_label_phrase(pair: Pair<'_, Rule>) -> Result<Vec<DbString>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::ident)
        .map(db_string_pair)
        .collect()
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
) -> Result<selene_core::DbString, ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::ident => label = Some(db_string_pair(child)?),
            rule if is_ddl_keyword_token(rule) => {}
            _ => return Err(unexpected_pair(child, "unexpected TRUNCATE TYPE child")),
        }
    }
    label.ok_or_else(|| ParserError::syntax(missing, source_span, None))
}

fn build_drop_type_parts(
    pair: Pair<'_, Rule>,
    missing: &'static str,
) -> Result<(selene_core::DbString, bool, DropBehavior), ParserError> {
    let source_span = span(&pair);
    let mut label = None;
    let mut if_exists = false;
    // Default behavior when the optional `RESTRICT | CASCADE` tail is absent.
    let mut behavior = DropBehavior::Restrict;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::if_exists => if_exists = true,
            Rule::ident => label = Some(db_string_pair(child)?),
            Rule::drop_behavior => behavior = build_drop_behavior(&child)?,
            rule if is_ddl_keyword_token(rule) => {}
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
            Rule::ident => name = Some(db_string_pair(child)?),
            Rule::typed_marker => {}
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
        let value = pair
            .into_inner()
            .find(|child| matches!(child.as_rule(), Rule::literal | Rule::record_constructor))
            .ok_or_else(|| {
                ParserError::syntax("DEFAULT constraint is missing value", source_span, None)
            })?;
        return Ok(TypePropertyConstraint::Default(
            expr::build_value_expr(value)?,
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
            .map(|child| db_string_pair(child))
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

fn build_label_list(pair: Pair<'_, Rule>) -> Result<Vec<selene_core::DbString>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::ident)
        .map(|child| db_string_pair(child))
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
