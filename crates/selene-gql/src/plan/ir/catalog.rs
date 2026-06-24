//! Catalog planner IR rows.

use selene_core::DbString;

use crate::{DropBehavior, EdgeEndpointSpec, GqlType, SourceSpan, ValidationMode};

use super::ProjectExpr;

/// Catalog operation produced by DDL lowering.
#[derive(Clone, Debug, PartialEq)]
pub enum CatalogOp {
    /// Create an open graph.
    CreateGraph {
        /// Graph name.
        name: DbString,
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
        name: DbString,
        /// Whether `IF EXISTS` was requested.
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// Create a node type.
    CreateNodeType {
        /// Node label.
        label: DbString,
        /// Resolved key label set in source order (ISO/IEC 39075:2024 §18.2 SR5,
        /// Feature GG21). Empty means the bare `:Name` element-type-name form
        /// whose singleton key label set is implied (Feature GG20) — the
        /// executor then keys on `LabelSet::single(label)`. A non-empty vector is
        /// an explicit `<node type key label set>` already validated against the
        /// IL003 cardinality cap at lowering.
        key_labels: Vec<DbString>,
        /// Whether `OR REPLACE` was requested.
        or_replace: bool,
        /// Whether `IF NOT EXISTS` was requested.
        if_not_exists: bool,
        /// Optional parent type.
        extends: Option<DbString>,
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
        label: DbString,
        /// Resolved key label set in source order (ISO/IEC 39075:2024 §18.3 SR6,
        /// Feature GG21). Empty means the bare `:Name` form whose singleton key
        /// label set is implied (Feature GG20). A non-empty vector is an explicit
        /// `<edge type key label set>` already validated against the IL003
        /// cardinality cap at lowering.
        key_labels: Vec<DbString>,
        /// Whether `OR REPLACE` was requested.
        or_replace: bool,
        /// Whether `IF NOT EXISTS` was requested.
        if_not_exists: bool,
        /// Optional parent type.
        extends: Option<DbString>,
        /// Optional endpoint declaration.
        endpoints: Option<EdgeEndpointSpec>,
        /// Property definitions.
        properties: Vec<PlannedTypePropertyDef>,
        /// Optional validation mode.
        validation_mode: Option<ValidationMode>,
        /// Source span.
        span: SourceSpan,
    },
    /// Alter an existing edge type through forward-only additive changes.
    AlterEdgeType {
        /// Edge type label.
        label: DbString,
        /// Optional widened endpoint declaration.
        endpoints: Option<EdgeEndpointSpec>,
        /// Property definitions to add.
        properties: Vec<PlannedTypePropertyDef>,
        /// Source span.
        span: SourceSpan,
    },
    /// Drop a node type.
    DropNodeType {
        /// Node label.
        label: DbString,
        /// Whether `IF EXISTS` was requested.
        if_exists: bool,
        /// `RESTRICT` (default) or `CASCADE` drop behavior.
        behavior: DropBehavior,
        /// Source span.
        span: SourceSpan,
    },
    /// Drop an edge type.
    DropEdgeType {
        /// Edge label.
        label: DbString,
        /// Whether `IF EXISTS` was requested.
        if_exists: bool,
        /// `RESTRICT` (default) or `CASCADE` drop behavior.
        behavior: DropBehavior,
        /// Source span.
        span: SourceSpan,
    },
    /// Truncate (bulk-delete instances of) a node type (`IM_TRUNCATE`).
    TruncateNodeType {
        /// Node label whose instances are removed.
        label: DbString,
        /// Source span.
        span: SourceSpan,
    },
    /// Truncate (bulk-delete instances of) an edge type (`IM_TRUNCATE`).
    TruncateEdgeType {
        /// Edge label whose instances are removed.
        label: DbString,
        /// Source span.
        span: SourceSpan,
    },
    /// Create a named property index.
    CreateIndex {
        /// Catalog index name.
        name: DbString,
        /// Node label.
        label: DbString,
        /// Property names in source order.
        properties: Vec<DbString>,
        /// Whether `IF NOT EXISTS` was requested.
        if_not_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// Drop a named property index.
    DropIndex {
        /// Catalog index name.
        name: DbString,
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
    pub name: DbString,
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
        name: Option<DbString>,
        /// Source span.
        span: SourceSpan,
    },
}
