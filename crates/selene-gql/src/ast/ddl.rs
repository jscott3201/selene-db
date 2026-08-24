//! Data-definition statement AST nodes.

use selene_core::DbString;

use crate::ast::{
    catalog_ref::CatalogObjectReference, expr::ValueExpr, span::SourceSpan, types::GqlType,
};

/// Data-definition statement.
///
/// The first four variants are database-catalog statements (ISO/IEC
/// 39075:2024 §12.2–§12.5). They carry unresolved references and are executed
/// by the database facade, not by the graph-local executor.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum DdlStatement {
    /// `CREATE SCHEMA [IF NOT EXISTS] <catalog schema parent and name>` (§12.2).
    CreateSchema {
        /// Absolute schema reference; the grammar requires the leading `/`.
        reference: CatalogObjectReference,
        /// `IF NOT EXISTS` (Feature GC02).
        if_not_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP SCHEMA [IF EXISTS] <catalog schema parent and name>` (§12.3).
    DropSchema {
        /// Absolute schema reference; the grammar requires the leading `/`.
        reference: CatalogObjectReference,
        /// `IF EXISTS` (Feature GC02).
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `CREATE [PROPERTY] GRAPH [IF NOT EXISTS] <reference> <open graph type>`
    /// (§12.4).
    ///
    /// Only the `<open graph type>` form (`[TYPED | ::] ANY [[PROPERTY]
    /// GRAPH]`, Feature GG01) is representable. The builder rejects every
    /// `<of graph type>` form (GG02/GG03/GG04) and `<graph source>` (GG05)
    /// with a feature-cited diagnostic, so no clause the facade cannot honor
    /// survives into the AST.
    CreateGraph {
        /// Absolute or current-schema-relative graph reference.
        reference: CatalogObjectReference,
        /// `OR REPLACE` (ISO/IEC 39075:2024 section 12.4): the facade drops an
        /// existing graph and creates the new one in a single publication.
        or_replace: bool,
        /// `IF NOT EXISTS` (Feature GC05).
        if_not_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP [PROPERTY] GRAPH [IF EXISTS] <reference>` (§12.5).
    DropGraph {
        /// Absolute or current-schema-relative graph reference.
        reference: CatalogObjectReference,
        /// `IF EXISTS` (Feature GC05).
        if_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `CREATE NODE TYPE`.
    CreateNodeType {
        /// Node label, stored without the source `:` prefix.
        label: DbString,
        /// Explicit `<node type key label set>` (Feature GG21), or `None` for the
        /// bare `:Name` element-type-name form whose key label set is implied
        /// (Feature GG20).
        key_label_set: Option<KeyLabelSet>,
        /// `OR REPLACE`.
        or_replace: bool,
        /// `IF NOT EXISTS`.
        if_not_exists: bool,
        /// Optional parent type.
        extends: Option<DbString>,
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
        label: DbString,
        /// Explicit `<edge type key label set>` (Feature GG21), or `None` for the
        /// bare `:Name` element-type-name form whose key label set is implied
        /// (Feature GG20).
        key_label_set: Option<KeyLabelSet>,
        /// `OR REPLACE`.
        or_replace: bool,
        /// `IF NOT EXISTS`.
        if_not_exists: bool,
        /// Optional parent type.
        extends: Option<DbString>,
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
    /// `ALTER NODE TYPE`.
    AlterNodeType {
        /// Node type label.
        label: DbString,
        /// Property definitions to add.
        properties: Vec<TypePropertyDef>,
        /// Source span.
        span: SourceSpan,
    },
    /// `ALTER EDGE TYPE`.
    AlterEdgeType {
        /// Edge type label.
        label: DbString,
        /// Optional widened endpoint declaration.
        endpoints: Option<EdgeEndpointSpec>,
        /// Optional property definitions to add.
        properties: Vec<TypePropertyDef>,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP NODE TYPE`.
    DropNodeType {
        /// Node label.
        label: DbString,
        /// `IF EXISTS`.
        if_exists: bool,
        /// `RESTRICT` (default) or `CASCADE` drop behavior.
        behavior: DropBehavior,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP EDGE TYPE`.
    DropEdgeType {
        /// Edge label.
        label: DbString,
        /// `IF EXISTS`.
        if_exists: bool,
        /// `RESTRICT` (default) or `CASCADE` drop behavior.
        behavior: DropBehavior,
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
        label: DbString,
        /// Source span.
        span: SourceSpan,
    },
    /// `TRUNCATE EDGE TYPE` (selene-db `IM_TRUNCATE` vendor extension).
    ///
    /// Bulk-removes every edge carrying `label` with an O(1) WAL write. Absent
    /// label is a clean no-op.
    TruncateEdgeType {
        /// Edge label whose instances are removed.
        label: DbString,
        /// Source span.
        span: SourceSpan,
    },
    /// `CREATE INDEX`.
    CreateIndex {
        /// Catalog index name.
        name: DbString,
        /// Node label, stored without the source `:` prefix.
        label: DbString,
        /// Property names in source order.
        properties: Vec<DbString>,
        /// `IF NOT EXISTS`.
        if_not_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP INDEX`.
    DropIndex {
        /// Catalog index name.
        name: DbString,
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
    /// Lists registered property indexes (built-in plus those created with
    /// `CREATE INDEX`).
    ShowIndexes(SourceSpan),
    /// `SHOW PROCEDURES`.
    ShowProcedures(SourceSpan),
}

impl DdlStatement {
    /// Return this statement's source span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::CreateSchema { span, .. }
            | Self::DropSchema { span, .. }
            | Self::CreateGraph { span, .. }
            | Self::DropGraph { span, .. }
            | Self::CreateNodeType { span, .. }
            | Self::CreateEdgeType { span, .. }
            | Self::AlterNodeType { span, .. }
            | Self::AlterEdgeType { span, .. }
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

/// `DROP NODE TYPE` / `DROP EDGE TYPE` drop behavior.
///
/// `Restrict` is the default when no behavior keyword is written. It is the
/// Seam-B fix from the deletion-reclamation audit (Item 3): dropping a type
/// whose instances still exist is rejected with `G2000` rather than silently
/// orphaning instances on a closed (GG02) graph. `Cascade` is the selene-db
/// `IM_DROP_CASCADE` vendor extension: it truncates the type's instances first,
/// then drops the type, atomically in one transaction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum DropBehavior {
    /// Reject the drop when instances or inbound type dependencies remain.
    Restrict,
    /// Truncate the type's instances, then drop the type (`IM_DROP_CASCADE`).
    Cascade,
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

/// Explicitly-written element type key label set (ISO/IEC 39075:2024 §18.2/18.3,
/// Feature GG21 "Explicit element type key label sets").
///
/// Present only when the source contains the explicit `<...type key label set>`
/// production (`[ <label set phrase> ] <implies>`, i.e. a `=>` marker). The bare
/// `:Name` element-type-name form (Feature GG20, key label set implied per §18.2
/// SR5c) leaves the owning statement's `key_label_set` field `None`.
///
/// `labels` carries the labels of the explicit `<label set phrase>` in source
/// order (the key label set per §18.2 SR5a). An empty `labels` is the bare
/// `<implies>` with no `<label set phrase>` (§18.2 SR5b — the empty key label
/// set, cardinality 0). `implied_labels` carries any separate post-`<implies>`
/// `<...type label set>` (the `:Person => :Employee` shape); selene-db defers
/// the union/containment-identification semantics this requires (§18.2 SR7/SR8)
/// to a later release and rejects a non-empty `implied_labels` with an honest
/// `FEATURE_NOT_SUPPORTED` (42N01) at plan time.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct KeyLabelSet {
    /// Labels of the explicit `<label set phrase>` in source order (empty for a
    /// bare `<implies>`).
    pub labels: Vec<DbString>,
    /// Labels of a separate post-`<implies>` `<...type label set>` (the deferred
    /// `:Key => :Implied` shape); empty in the supported singleton form.
    pub implied_labels: Vec<DbString>,
    /// Source span of the key-label-set production.
    pub span: SourceSpan,
}

/// Edge endpoint declaration.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EdgeEndpointSpec {
    /// Source node labels.
    pub from_labels: Vec<DbString>,
    /// Target node labels.
    pub to_labels: Vec<DbString>,
    /// Source span.
    pub span: SourceSpan,
}

/// One type property definition.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct TypePropertyDef {
    /// Property name.
    pub name: DbString,
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
        name: Option<DbString>,
        /// Source span.
        span: SourceSpan,
    },
}

impl TypePropertyConstraint {
    /// Return this constraint's source span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::NotNull(span)
            | Self::Default(_, span)
            | Self::Immutable(span)
            | Self::Unique(span) => *span,
            Self::Indexed { span, .. } => *span,
        }
    }
}
