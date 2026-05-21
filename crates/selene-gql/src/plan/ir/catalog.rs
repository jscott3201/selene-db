//! Catalog planner IR rows.

use selene_core::IStr;

use crate::{EdgeEndpointSpec, GqlType, SourceSpan, ValidationMode};

use super::ProjectExpr;

/// Catalog operation produced by DDL lowering.
#[derive(Clone, Debug, PartialEq)]
pub enum CatalogOp {
    /// Create an open graph.
    CreateGraph {
        /// Graph name.
        name: IStr,
        /// Whether `OR REPLACE` was requested.
        or_replace: bool,
        /// Whether `IF NOT EXISTS` was requested.
        if_not_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// Drop a graph.
    DropGraph {
        /// Graph name.
        name: IStr,
        /// Whether `IF EXISTS` was requested.
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// Create a node type.
    CreateNodeType {
        /// Node label.
        label: IStr,
        /// Whether `OR REPLACE` was requested.
        or_replace: bool,
        /// Whether `IF NOT EXISTS` was requested.
        if_not_exists: bool,
        /// Optional parent type.
        extends: Option<IStr>,
        /// Property definitions.
        properties: Vec<PlannedTypePropertyDef>,
        /// Optional validation mode.
        validation_mode: Option<ValidationMode>,
        /// Source span.
        span: SourceSpan,
    },
    /// Create an edge type.
    CreateEdgeType {
        /// Edge label.
        label: IStr,
        /// Whether `OR REPLACE` was requested.
        or_replace: bool,
        /// Whether `IF NOT EXISTS` was requested.
        if_not_exists: bool,
        /// Optional endpoint declaration.
        endpoints: Option<EdgeEndpointSpec>,
        /// Property definitions.
        properties: Vec<PlannedTypePropertyDef>,
        /// Optional validation mode.
        validation_mode: Option<ValidationMode>,
        /// Source span.
        span: SourceSpan,
    },
    /// Drop a node type.
    DropNodeType {
        /// Node label.
        label: IStr,
        /// Whether `IF EXISTS` was requested.
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// Drop an edge type.
    DropEdgeType {
        /// Edge label.
        label: IStr,
        /// Whether `IF EXISTS` was requested.
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// Show node types.
    ShowNodeTypes(SourceSpan),
    /// Show edge types.
    ShowEdgeTypes(SourceSpan),
    /// Show built-in property indexes.
    ShowIndexes(SourceSpan),
    /// Show registered procedures.
    ShowProcedures(SourceSpan),
}

/// Planner-side type property definition.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedTypePropertyDef {
    /// Property name.
    pub name: IStr,
    /// Declared GQL type.
    pub gql_type: GqlType,
    /// Planned constraints.
    pub constraints: Vec<PlannedTypePropertyConstraint>,
    /// Source span.
    pub span: SourceSpan,
}

/// Planner-side type property constraint.
#[derive(Clone, Debug, PartialEq)]
pub enum PlannedTypePropertyConstraint {
    /// `NOT NULL`.
    NotNull(SourceSpan),
    /// `DEFAULT expr`.
    Default(ProjectExpr, SourceSpan),
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
    /// `INTERVAL value`.
    Interval(IStr, SourceSpan),
    /// `ENCODING name`.
    Encoding(IStr, SourceSpan),
}
