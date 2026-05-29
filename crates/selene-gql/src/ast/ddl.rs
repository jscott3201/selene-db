//! Data-definition statement AST nodes.

use selene_core::IStr;

use crate::ast::{expr::ValueExpr, span::SourceSpan, types::GqlType};

/// Data-definition statement.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum DdlStatement {
    /// `CREATE GRAPH`.
    CreateGraph {
        /// Graph name.
        name: IStr,
        /// `OR REPLACE`.
        or_replace: bool,
        /// `IF NOT EXISTS`.
        if_not_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP GRAPH`.
    DropGraph {
        /// Graph name.
        name: IStr,
        /// `IF EXISTS`.
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `CREATE NODE TYPE`.
    CreateNodeType {
        /// Node label, stored without the source `:` prefix.
        label: IStr,
        /// `OR REPLACE`.
        or_replace: bool,
        /// `IF NOT EXISTS`.
        if_not_exists: bool,
        /// Optional parent type.
        extends: Option<IStr>,
        /// Property definitions.
        properties: Vec<TypePropertyDef>,
        /// Optional validation mode.
        ///
        /// Defaults to strict validation when omitted.
        validation_mode: Option<ValidationMode>,
        /// Source span.
        span: SourceSpan,
    },
    /// `CREATE EDGE TYPE`.
    CreateEdgeType {
        /// Edge label, stored without the source `:` prefix.
        label: IStr,
        /// `OR REPLACE`.
        or_replace: bool,
        /// `IF NOT EXISTS`.
        if_not_exists: bool,
        /// Optional parent type.
        extends: Option<IStr>,
        /// Optional endpoint declaration.
        endpoints: Option<EdgeEndpointSpec>,
        /// Property definitions.
        properties: Vec<TypePropertyDef>,
        /// Optional validation mode.
        ///
        /// Defaults to strict validation when omitted.
        validation_mode: Option<ValidationMode>,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP NODE TYPE`.
    DropNodeType {
        /// Node label.
        label: IStr,
        /// `IF EXISTS`.
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP EDGE TYPE`.
    DropEdgeType {
        /// Edge label.
        label: IStr,
        /// `IF EXISTS`.
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `TRUNCATE NODE TYPE` (selene-db `IM_TRUNCATE` vendor extension).
    ///
    /// Bulk-removes every node carrying `label` and all incident edges,
    /// observationally identical to `MATCH (n:label) DETACH DELETE n` but with an
    /// O(1) WAL write (deletion-reclamation audit Item 11). An absent label is a
    /// clean no-op; no `IF EXISTS` is needed.
    TruncateNodeType {
        /// Node label whose instances are removed.
        label: IStr,
        /// Source span.
        span: SourceSpan,
    },
    /// `TRUNCATE EDGE TYPE` (selene-db `IM_TRUNCATE` vendor extension).
    ///
    /// Bulk-removes every edge carrying `label` with an O(1) WAL write. Absent
    /// label is a clean no-op.
    TruncateEdgeType {
        /// Edge label whose instances are removed.
        label: IStr,
        /// Source span.
        span: SourceSpan,
    },
    /// `CREATE INDEX`.
    CreateIndex {
        /// Catalog index name.
        name: IStr,
        /// Node label, stored without the source `:` prefix.
        label: IStr,
        /// Property names in source order.
        properties: Vec<IStr>,
        /// `IF NOT EXISTS`.
        if_not_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP INDEX`.
    DropIndex {
        /// Catalog index name.
        name: IStr,
        /// `IF EXISTS`.
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `SHOW NODE TYPES`.
    ShowNodeTypes(SourceSpan),
    /// `SHOW EDGE TYPES`.
    ShowEdgeTypes(SourceSpan),
    /// `SHOW INDEXES`.
    ///
    /// Lists built-in property indexes only. Vector indexes remain exposed via
    /// `CALL vector.list_indexes()`.
    ShowIndexes(SourceSpan),
    /// `SHOW PROCEDURES`.
    ShowProcedures(SourceSpan),
}

impl DdlStatement {
    /// Return this statement's source span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::CreateGraph { span, .. }
            | Self::DropGraph { span, .. }
            | Self::CreateNodeType { span, .. }
            | Self::CreateEdgeType { span, .. }
            | Self::DropNodeType { span, .. }
            | Self::DropEdgeType { span, .. }
            | Self::TruncateNodeType { span, .. }
            | Self::TruncateEdgeType { span, .. }
            | Self::CreateIndex { span, .. }
            | Self::DropIndex { span, .. }
            | Self::ShowNodeTypes(span)
            | Self::ShowEdgeTypes(span)
            | Self::ShowIndexes(span)
            | Self::ShowProcedures(span) => *span,
        }
    }
}

/// Type-validation mode.
///
/// Closed-graph write validation mode for catalog type declarations.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum ValidationMode {
    /// Reject violations.
    Strict,
    /// Warn on violations.
    Warn,
}

/// Edge endpoint declaration.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EdgeEndpointSpec {
    /// Source node labels.
    pub from_labels: Vec<IStr>,
    /// Target node labels.
    pub to_labels: Vec<IStr>,
    /// Source span.
    pub span: SourceSpan,
}

/// One type property definition.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct TypePropertyDef {
    /// Property name.
    pub name: IStr,
    /// GQL value type.
    pub gql_type: GqlType,
    /// Property constraints.
    pub constraints: Vec<TypePropertyConstraint>,
    /// Source span.
    pub span: SourceSpan,
}

/// Constraint attached to a type property.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum TypePropertyConstraint {
    /// `NOT NULL`.
    ///
    /// This is the only property constraint that currently makes a catalog
    /// property required at runtime; `DEFAULT` alone leaves the property
    /// nullable.
    NotNull(SourceSpan),
    /// `DEFAULT expr`.
    ///
    /// `DEFAULT` is independent from `NOT NULL`; DEFAULT alone does not imply
    /// required.
    Default(ValueExpr, SourceSpan),
    /// `IMMUTABLE`.
    Immutable(SourceSpan),
    /// `UNIQUE`.
    Unique(SourceSpan),
    /// `INDEXED [AS name]`.
    Indexed {
        /// Optional explicit index name.
        name: Option<IStr>,
        /// Source span.
        span: SourceSpan,
    },
    /// `SEARCHABLE`.
    Searchable(SourceSpan),
    /// `DICTIONARY`.
    Dictionary(SourceSpan),
    /// `FILL name`.
    Fill(IStr, SourceSpan),
    /// `INTERVAL 'duration'`.
    Interval(IStr, SourceSpan),
    /// `ENCODING name`.
    Encoding(IStr, SourceSpan),
}

impl TypePropertyConstraint {
    /// Return this constraint's source span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::NotNull(span)
            | Self::Default(_, span)
            | Self::Immutable(span)
            | Self::Unique(span)
            | Self::Searchable(span)
            | Self::Dictionary(span)
            | Self::Fill(_, span)
            | Self::Interval(_, span)
            | Self::Encoding(_, span) => *span,
            Self::Indexed { span, .. } => *span,
        }
    }
}
