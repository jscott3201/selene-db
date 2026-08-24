//! Router for GQL database-catalog statements executed by the compatibility
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
//! The current working schema of the compatibility session is fixed to the
//! bootstrap schema `/selene/public`. This is the ISO `INI_SCHEMA` shape of
//! §22.1 for a session that never executed `SESSION SET SCHEMA`; there is no
//! setter because the facade never accepts configuration it would ignore.
//! M03-PR01's session context replaces this constant.
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
//! of another kind is `42002`, and a reference that resolves to the protected
//! bootstrap graph is `42000`: the factory-reset bridge below applies to
//! `DROP GRAPH` only, so the bootstrap graph can be reset but never replaced.
//! Graph-type replacement uses the same policy and the full
//! [`Catalog::drop_graph_type`] RESTRICT admission. A referenced type is
//! `G1000`; an accepted replacement receives a fresh identity in one outer
//! state swap.
//!
//! # Bootstrap `DROP GRAPH` bridge
//!
//! A `DROP GRAPH` whose reference resolves to the protected bootstrap graph
//! (`/selene/public/default`, however it is spelled) keeps the compatibility
//! session's pre-existing factory reset through the lower engine. The
//! decision is by resolved stable identity, not spelling. Every other
//! reference goes to [`Catalog::drop_graph`]. M02-PR05 deletes this bridge
//! together with the bootstrap catalog.
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
    GqlStatus, GraphTypeDefinition, NodeTypeDefinition, ObjectPath, PathSegment, Result,
    SchemaPath,
    database::{DatabaseInner, bootstrap_graph_id},
};

/// Execute one database-catalog command for the compatibility session.
pub(crate) fn execute(
    inner: &Arc<DatabaseInner>,
    command: DatabaseCatalogCommand,
    source: &str,
) -> Result<ExecutionOutcome> {
    let catalog = Catalog::new(Arc::clone(inner));
    let outcome = match command {
        DatabaseCatalogCommand::CreateSchema {
            reference,
            if_not_exists,
            ..
        } => {
            let path = resolve_schema(inner, &reference)?;
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
            let path = resolve_schema(inner, &reference)?;
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
            let path = resolve_graph(inner, &reference)?;
            let graph_type = graph_type
                .as_ref()
                .map(|reference| resolve_graph(inner, reference))
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
            let path = resolve_graph(inner, &reference)?;
            if resolves_to_bootstrap_graph(&catalog, &path) {
                return inner.execute_graph(bootstrap_graph_id(), &path, source, false);
            }
            match catalog.drop_graph(&path, drop_policy(if_exists))? {
                DropOutcome::Dropped(_) => omitted(),
                // Section 12.5 GR1: a completion condition, not an exception.
                DropOutcome::NotFound => ExecutionOutcome::OmittedResult {
                    status: GqlStatus::GRAPH_DOES_NOT_EXIST,
                },
            }
        }
        DatabaseCatalogCommand::CreateGraphType {
            reference,
            definition,
            or_replace,
            if_not_exists,
            ..
        } => {
            let path = resolve_graph(inner, &reference)?;
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
            let path = resolve_graph(inner, &reference)?;
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
const fn omitted() -> ExecutionOutcome {
    ExecutionOutcome::OmittedResult {
        status: GqlStatus::SUCCESSFUL_COMPLETION_OMITTED_RESULT,
    }
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

/// Decide the bootstrap bridge by stable identity.
///
/// The bootstrap graph is protected from drop and rename, so a lookup outside
/// the lifecycle lock cannot be invalidated by a concurrent lifecycle change:
/// the identity at this path is either the fixed bootstrap ID or it is not.
fn resolves_to_bootstrap_graph(catalog: &Catalog, path: &ObjectPath) -> bool {
    catalog
        .snapshot()
        .resolve_graph(path)
        .is_ok_and(|descriptor| descriptor.id.get() == bootstrap_graph_id().get())
}

fn resolve_schema(inner: &DatabaseInner, reference: &CatalogObjectReference) -> Result<SchemaPath> {
    match reference.segments.as_slice() {
        [schema] if reference.absolute => {
            Ok(SchemaPath::new(catalog_segment(inner)?, segment(schema)?))
        }
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

fn resolve_graph(inner: &DatabaseInner, reference: &CatalogObjectReference) -> Result<ObjectPath> {
    match reference.segments.as_slice() {
        [graph] if !reference.absolute => {
            let schema = current_working_schema(inner)?;
            Ok(ObjectPath::new(
                schema.catalog.clone(),
                schema.schema.clone(),
                segment(graph)?,
            ))
        }
        [schema, graph] => Ok(ObjectPath::new(
            catalog_segment(inner)?,
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

/// The fixed current working schema of the compatibility session.
fn current_working_schema(inner: &DatabaseInner) -> Result<SchemaPath> {
    Ok(SchemaPath::new(
        catalog_segment(inner)?,
        PathSegment::regular(inner.bootstrap.schema_name())?,
    ))
}

fn catalog_segment(inner: &DatabaseInner) -> Result<PathSegment> {
    PathSegment::regular(inner.bootstrap.catalog_name())
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
