//! Storage-neutral database-catalog commands handed to the database facade.
//!
//! Schema, graph, and graph-type lifecycle state belongs to the database
//! catalog, not the lower engine. The parser and planner reduce those
//! statements to a [`DatabaseCatalogCommand`]; the database facade resolves
//! the carried references against its typed catalog paths and dispatches to the
//! same lifecycle service Rust callers use. Commands retain unresolved,
//! form-tagged spellings rather than resolved IDs. Resolution happens under the
//! facade's lifecycle writer lock, preventing a concurrent drop/recreate window.
//!
//! A bare lower-engine session cannot honor these commands and reports a
//! structured implementation-defined error instead of a silent no-op. The one
//! pre-existing exception is `DROP GRAPH`, which the lower engine still
//! executes as the `IM_DROP_GRAPH` factory reset of its single bound graph;
//! the facade routes only the protected bootstrap graph to that path.

use crate::{
    CatalogGraphTypeDefinition, DdlStatement, SourceSpan,
    ast::catalog_ref::{CatalogObjectReference, CatalogPathSegment},
};

/// One database-catalog statement reduced to its storage-neutral effect.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DatabaseCatalogCommand {
    /// `CREATE SCHEMA [IF NOT EXISTS] <catalog schema parent and name>`.
    CreateSchema {
        /// Absolute schema reference.
        reference: CatalogObjectReference,
        /// Whether `IF NOT EXISTS` was written.
        if_not_exists: bool,
        /// Span of the whole statement.
        span: SourceSpan,
    },
    /// `DROP SCHEMA [IF EXISTS] <catalog schema parent and name>`.
    DropSchema {
        /// Absolute schema reference.
        reference: CatalogObjectReference,
        /// Whether `IF EXISTS` was written.
        if_exists: bool,
        /// Span of the whole statement.
        span: SourceSpan,
    },
    /// `CREATE { [PROPERTY] GRAPH [IF NOT EXISTS] | OR REPLACE [PROPERTY]
    /// GRAPH } <reference> <graph type>`.
    ///
    /// The graph type is open when `graph_type` is `None`, or a named closed
    /// graph type otherwise. Inline/LIKE types and graph sources are rejected
    /// before lowering. The grammar makes `IF NOT EXISTS` and `OR REPLACE`
    /// alternatives, so at most one flag is set.
    CreateGraph {
        /// Absolute or current-schema-relative graph reference.
        reference: CatalogObjectReference,
        /// Whether `OR REPLACE` was written (ISO/IEC 39075:2024 section 12.4
        /// GR2: an existing graph is dropped before the new one is created).
        or_replace: bool,
        /// Whether `IF NOT EXISTS` was written.
        if_not_exists: bool,
        /// Named closed graph type, or `None` for an open graph.
        graph_type: Option<CatalogObjectReference>,
        /// Span of the whole statement.
        span: SourceSpan,
    },
    /// `DROP [PROPERTY] GRAPH [IF EXISTS] <reference>`.
    DropGraph {
        /// Absolute or current-schema-relative graph reference.
        reference: CatalogObjectReference,
        /// Whether `IF EXISTS` was written.
        if_exists: bool,
        /// Span of the whole statement.
        span: SourceSpan,
    },
    /// `CREATE [PROPERTY] GRAPH TYPE` with a bounded nested definition.
    CreateGraphType {
        /// Absolute or current-schema-relative graph-type reference.
        reference: CatalogObjectReference,
        /// Property-free named node definitions with form-tagged names.
        definition: CatalogGraphTypeDefinition,
        /// Whether `OR REPLACE` was written.
        or_replace: bool,
        /// Whether `IF NOT EXISTS` was written.
        if_not_exists: bool,
        /// Span of the whole statement.
        span: SourceSpan,
    },
    /// `DROP [PROPERTY] GRAPH TYPE [IF EXISTS] <reference>`.
    DropGraphType {
        /// Absolute or current-schema-relative graph-type reference.
        reference: CatalogObjectReference,
        /// Whether `IF EXISTS` was written.
        if_exists: bool,
        /// Span of the whole statement.
        span: SourceSpan,
    },
}

impl DatabaseCatalogCommand {
    /// Reduce a parsed DDL statement to its command, or `None` when the
    /// statement is graph-local DDL that the lower engine executes itself.
    #[must_use]
    pub fn from_ddl(statement: &DdlStatement) -> Option<Self> {
        match statement {
            DdlStatement::CreateSchema {
                reference,
                if_not_exists,
                span,
            } => Some(Self::CreateSchema {
                reference: reference.clone(),
                if_not_exists: *if_not_exists,
                span: *span,
            }),
            DdlStatement::DropSchema {
                reference,
                if_exists,
                span,
            } => Some(Self::DropSchema {
                reference: reference.clone(),
                if_exists: *if_exists,
                span: *span,
            }),
            DdlStatement::CreateGraph {
                reference,
                or_replace,
                if_not_exists,
                graph_type,
                span,
            } => Some(Self::CreateGraph {
                reference: reference.clone(),
                or_replace: *or_replace,
                if_not_exists: *if_not_exists,
                graph_type: graph_type.clone(),
                span: *span,
            }),
            DdlStatement::DropGraph {
                reference,
                if_exists,
                span,
            } => Some(Self::DropGraph {
                reference: reference.clone(),
                if_exists: *if_exists,
                span: *span,
            }),
            DdlStatement::CreateGraphType {
                reference,
                definition,
                or_replace,
                if_not_exists,
                span,
            } => Some(Self::CreateGraphType {
                reference: reference.clone(),
                definition: definition.clone(),
                or_replace: *or_replace,
                if_not_exists: *if_not_exists,
                span: *span,
            }),
            DdlStatement::DropGraphType {
                reference,
                if_exists,
                span,
            } => Some(Self::DropGraphType {
                reference: reference.clone(),
                if_exists: *if_exists,
                span: *span,
            }),
            _ => None,
        }
    }

    /// Return the carried object reference.
    #[must_use]
    pub const fn reference(&self) -> &CatalogObjectReference {
        match self {
            Self::CreateSchema { reference, .. }
            | Self::DropSchema { reference, .. }
            | Self::CreateGraph { reference, .. }
            | Self::DropGraph { reference, .. }
            | Self::CreateGraphType { reference, .. }
            | Self::DropGraphType { reference, .. } => reference,
        }
    }

    /// Return the span of the whole statement.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::CreateSchema { span, .. }
            | Self::DropSchema { span, .. }
            | Self::CreateGraph { span, .. }
            | Self::DropGraph { span, .. }
            | Self::CreateGraphType { span, .. }
            | Self::DropGraphType { span, .. } => *span,
        }
    }

    /// Return the statement verb for diagnostics and plan summaries.
    #[must_use]
    pub const fn verb(&self) -> &'static str {
        match self {
            Self::CreateSchema { .. } => "CREATE SCHEMA",
            Self::DropSchema { .. } => "DROP SCHEMA",
            Self::CreateGraph { .. } => "CREATE GRAPH",
            Self::DropGraph { .. } => "DROP GRAPH",
            Self::CreateGraphType { .. } => "CREATE GRAPH TYPE",
            Self::DropGraphType { .. } => "DROP GRAPH TYPE",
        }
    }

    /// Return the segments of the carried reference.
    #[must_use]
    pub fn segments(&self) -> &[CatalogPathSegment] {
        &self.reference().segments
    }
}
