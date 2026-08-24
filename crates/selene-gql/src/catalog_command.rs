//! Storage-neutral database-catalog commands handed to the database facade.
//!
//! `CREATE/DROP SCHEMA` and `CREATE/DROP GRAPH` change catalog state that the
//! lower engine does not own. The parser and planner reduce such a statement
//! to a [`DatabaseCatalogCommand`]; the database facade resolves the carried
//! reference against its typed catalog paths and dispatches to the same
//! lifecycle service Rust callers use. The command deliberately carries
//! unresolved, form-tagged spellings rather than resolved IDs: resolution
//! happens under the facade's lifecycle writer lock so that a concurrent drop
//! or recreate between parse and dispatch cannot be observed (no TOCTOU
//! window), exactly as for Rust callers.
//!
//! A bare lower-engine session cannot honor these commands and reports a
//! structured implementation-defined error instead of a silent no-op. The one
//! pre-existing exception is `DROP GRAPH`, which the lower engine still
//! executes as the `IM_DROP_GRAPH` factory reset of its single bound graph;
//! the facade routes only the protected bootstrap graph to that path.

use crate::{
    DdlStatement, SourceSpan,
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
    /// `CREATE [PROPERTY] GRAPH [IF NOT EXISTS] <reference> <open graph type>`.
    ///
    /// Only the open graph type form reaches this command; every `<of graph
    /// type>` and `<graph source>` clause is rejected by the parser.
    CreateGraph {
        /// Absolute or current-schema-relative graph reference.
        reference: CatalogObjectReference,
        /// Whether `IF NOT EXISTS` was written.
        if_not_exists: bool,
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
                if_not_exists,
                span,
                ..
            } => Some(Self::CreateGraph {
                reference: reference.clone(),
                if_not_exists: *if_not_exists,
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
            | Self::DropGraph { reference, .. } => reference,
        }
    }

    /// Return the span of the whole statement.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::CreateSchema { span, .. }
            | Self::DropSchema { span, .. }
            | Self::CreateGraph { span, .. }
            | Self::DropGraph { span, .. } => *span,
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
        }
    }

    /// Return the segments of the carried reference.
    #[must_use]
    pub fn segments(&self) -> &[CatalogPathSegment] {
        &self.reference().segments
    }
}
