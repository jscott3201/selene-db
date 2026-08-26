//! Router for GQL database-catalog statements executed by a selected
//! [`Session`](crate::Session).
//!
//! The lower engine reduces schema, graph, and graph-type lifecycle statements
//! to a storage-neutral [`DatabaseCatalogCommand`] and hands it back before any
//! execution context exists. This module resolves the carried references and
//! node definitions to facade-owned path/definition types, then dispatches to
//! the same [`Catalog`] methods Rust callers use.
//!
//! # Reference resolution
//!
//! ISO absolute references never spell the catalog; facade paths do. The
//! rules are:
//!
//! - schema reference `/x` → `/<catalog>/x`;
//! - absolute graph or graph-type reference `/x/g` → `/<catalog>/x/g`;
//! - relative graph or graph-type reference `g` →
//!   `/<catalog>/<current working schema>/g`
//!   (ISO/IEC 39075:2024 §17.2 SR2a);
//! - any other segment count is an invalid reference (`42002`): the facade
//!   catalog has directory depth 0 (IL020), and its root is a directory, not a
//!   schema (§17.1 SR2a).
//!
//! The selected graph's schema is the current working schema. The facade has no
//! schema-switching command or persistent session context; M03 owns those.
//!
//! Each segment becomes a [`PathSegment`] through the regular or delimited
//! constructor. That constructor is the only name-validation choke point; the
//! parser tags forms and decodes spellings but validates nothing.
//!
//! Resolution to stable IDs happens inside `Catalog::*` under the lifecycle
//! writer mutex, exactly as for Rust callers. The plan's "DDL execution
//! receives resolved paths/IDs" is deliberately read as typed paths: passing
//! pre-resolved IDs from a lock-free lookup would open a window between the
//! lookup and the lifecycle lock in which a concurrent drop and same-path
//! recreate could be observed.
//!
//! # `CREATE OR REPLACE GRAPH`
//!
//! `OR REPLACE` maps to [`CreatePolicy::OrReplace`]: [`Catalog::create_graph`]
//! drops an existing graph under the `DROP GRAPH` admission and publishes the
//! replacement in one state swap (§12.4 GR2), completing with `00001` whether
//! the graph was created or replaced. A nonempty graph is `G1000`, an object
//! of another kind is `42002`.
//! Graph-type replacement uses the same policy and the full
//! [`Catalog::drop_graph_type`] RESTRICT admission. A referenced type is
//! `G1000`; an accepted replacement receives a fresh identity in one outer
//! state swap.
//!
//! # Lock order
//!
//! This module is entered only after the graph request lease used for
//! parsing has been released. `Catalog::drop_graph` takes the lifecycle
//! writer and then the target's lifecycle write lease, so calling it under a
//! read lease on the same graph would self-deadlock; the test-only lease
//! accounting in [`DatabaseInner`] turns that mistake into a panic.

use std::{fmt::Write as _, sync::Arc};

use selene_gql::{
    CatalogGraphTypeDefinition, CatalogObjectReference, CatalogPathSegment, DatabaseCatalogCommand,
};

use crate::{
    Catalog, CreateOutcome, CreatePolicy, DropOutcome, DropPolicy, Error, ExecutionOutcome,
    GraphTypeDefinition, NodeTypeDefinition, ObjectPath, PathSegment, Result, SchemaPath,
    database::DatabaseInner,
};

/// Execute one database-catalog command for a selected session.
pub(crate) fn execute(
    inner: &Arc<DatabaseInner>,
    current_schema: &SchemaPath,
    command: DatabaseCatalogCommand,
) -> Result<ExecutionOutcome> {
    let catalog = Catalog::new(Arc::clone(inner));
    let outcome = match command {
        DatabaseCatalogCommand::CreateSchema {
            reference,
            if_not_exists,
            ..
        } => {
            let path = resolve_schema(current_schema, &reference)?;
            match catalog.create_schema(&path, create_policy(if_not_exists))? {
                // Schemas have no OR REPLACE form; the arm is exhaustive only.
                CreateOutcome::Created(_)
                | CreateOutcome::AlreadyExists(_)
                | CreateOutcome::Replaced { .. } => omitted(),
            }
        }
        DatabaseCatalogCommand::DropSchema {
            reference,
            if_exists,
            ..
        } => {
            let path = resolve_schema(current_schema, &reference)?;
            match catalog.drop_schema(&path, drop_policy(if_exists))? {
                // ISO/IEC 39075:2024 section 12.3 GR1 defines no warning for
                // an absent schema under IF EXISTS.
                DropOutcome::Dropped(_) | DropOutcome::NotFound => omitted(),
            }
        }
        DatabaseCatalogCommand::CreateGraph {
            reference,
            or_replace,
            if_not_exists,
            graph_type,
            ..
        } => {
            let path = resolve_graph(current_schema, &reference)?;
            let graph_type = graph_type
                .as_ref()
                .map(|reference| resolve_graph(current_schema, reference))
                .transpose()?;
            let policy = if or_replace {
                CreatePolicy::OrReplace
            } else {
                create_policy(if_not_exists)
            };
            match catalog.create_graph(&path, graph_type.as_ref(), policy)? {
                // Section 12.4 GR2: OR REPLACE effectively executes DROP GRAPH
                // first; the whole statement still completes as one 00001.
                CreateOutcome::Created(_)
                | CreateOutcome::AlreadyExists(_)
                | CreateOutcome::Replaced { .. } => omitted(),
            }
        }
        DatabaseCatalogCommand::DropGraph {
            reference,
            if_exists,
            ..
        } => {
            let path = resolve_graph(current_schema, &reference)?;
            match catalog.drop_graph(&path, drop_policy(if_exists))? {
                DropOutcome::Dropped(_) => omitted(),
                // Section 12.5 GR1: a completion condition, not an exception.
                DropOutcome::NotFound => ExecutionOutcome::GRAPH_NOT_FOUND_OMITTED,
            }
        }
        DatabaseCatalogCommand::CreateGraphType {
            reference,
            definition,
            or_replace,
            if_not_exists,
            ..
        } => {
            let path = resolve_graph(current_schema, &reference)?;
            let definition = graph_type_definition(definition)?;
            let policy = if or_replace {
                CreatePolicy::OrReplace
            } else {
                create_policy(if_not_exists)
            };
            match catalog.create_graph_type(&path, definition, policy)? {
                CreateOutcome::Created(_)
                | CreateOutcome::AlreadyExists(_)
                | CreateOutcome::Replaced { .. } => omitted(),
            }
        }
        DatabaseCatalogCommand::DropGraphType {
            reference,
            if_exists,
            ..
        } => {
            let path = resolve_graph(current_schema, &reference)?;
            match catalog.drop_graph_type(&path, drop_policy(if_exists))? {
                DropOutcome::Dropped(_) | DropOutcome::NotFound => omitted(),
            }
        }
        _ => return Err(Error::unsupported_engine_outcome()),
    };
    Ok(outcome)
}

/// Section 12.1 GR2: a successful catalog-modifying statement completes with
/// an omitted result.
fn omitted() -> ExecutionOutcome {
    ExecutionOutcome::SUCCESSFUL_OMITTED
}

const fn create_policy(if_not_exists: bool) -> CreatePolicy {
    if if_not_exists {
        CreatePolicy::IfNotExists
    } else {
        CreatePolicy::Strict
    }
}

const fn drop_policy(if_exists: bool) -> DropPolicy {
    if if_exists {
        DropPolicy::IfExists
    } else {
        DropPolicy::Strict
    }
}

fn resolve_schema(
    current_schema: &SchemaPath,
    reference: &CatalogObjectReference,
) -> Result<SchemaPath> {
    match reference.segments.as_slice() {
        [schema] if reference.absolute => Ok(SchemaPath::new(
            current_schema.catalog().clone(),
            segment(schema)?,
        )),
        [_] => Err(invalid_reference(
            reference,
            "a schema reference must be absolute",
        )),
        _ => Err(invalid_reference(
            reference,
            "the catalog has no child directories (maximum directory depth 0)",
        )),
    }
}

fn resolve_graph(
    current_schema: &SchemaPath,
    reference: &CatalogObjectReference,
) -> Result<ObjectPath> {
    match reference.segments.as_slice() {
        [graph] if !reference.absolute => Ok(ObjectPath::new(
            current_schema.catalog.clone(),
            current_schema.schema.clone(),
            segment(graph)?,
        )),
        [schema, graph] => Ok(ObjectPath::new(
            current_schema.catalog().clone(),
            segment(schema)?,
            segment(graph)?,
        )),
        [_] => Err(invalid_reference(
            reference,
            "the catalog root is a directory, not a schema, so `/name` names no graph",
        )),
        _ => Err(invalid_reference(
            reference,
            "the catalog has no child directories (maximum directory depth 0)",
        )),
    }
}

/// The single name-validation choke point for GQL-originated segments.
fn segment(segment: &CatalogPathSegment) -> Result<PathSegment> {
    match segment.form {
        selene_gql::IdentifierForm::Regular => PathSegment::regular(segment.name.as_str()),
        selene_gql::IdentifierForm::Delimited => PathSegment::delimited(segment.name.as_str()),
    }
}

fn graph_type_definition(source: CatalogGraphTypeDefinition) -> Result<GraphTypeDefinition> {
    let mut builder = GraphTypeDefinition::builder();
    for node in source.node_types {
        let name = segment(&node.name)?;
        builder = builder.with_node_type(NodeTypeDefinition::new(name.clone(), vec![name])?);
    }
    builder.build()
}

fn invalid_reference(reference: &CatalogObjectReference, detail: &str) -> Error {
    Error::invalid_reference(&render_reference(reference), detail)
}

/// Render an unresolved reference for diagnostics, quoting delimited
/// segments the way validated paths are displayed.
fn render_reference(reference: &CatalogObjectReference) -> String {
    let mut text = String::new();
    for (index, segment) in reference.segments.iter().enumerate() {
        if reference.absolute || index > 0 {
            text.push('/');
        }
        match segment.form {
            selene_gql::IdentifierForm::Regular => text.push_str(segment.name.as_str()),
            selene_gql::IdentifierForm::Delimited => {
                let _ = write!(text, "`{}`", segment.name.as_str().replace('`', "``"));
            }
        }
    }
    text
}
