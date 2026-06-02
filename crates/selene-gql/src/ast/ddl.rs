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
        /// Explicit `<node type key label set>` (Feature GG21), or `None` for the
        /// bare `:Name` element-type-name form whose key label set is implied
        /// (Feature GG20).
        key_label_set: Option<KeyLabelSet>,
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
        /// Explicit `<edge type key label set>` (Feature GG21), or `None` for the
        /// bare `:Name` element-type-name form whose key label set is implied
        /// (Feature GG20).
        key_label_set: Option<KeyLabelSet>,
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
        /// `RESTRICT` (default) or `CASCADE` drop behavior.
        behavior: DropBehavior,
        /// Source span.
        span: SourceSpan,
    },
    /// `DROP EDGE TYPE`.
    DropEdgeType {
        /// Edge label.
        label: IStr,
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
    pub labels: Vec<IStr>,
    /// Labels of a separate post-`<implies>` `<...type label set>` (the deferred
    /// `:Key => :Implied` shape); empty in the supported singleton form.
    pub implied_labels: Vec<IStr>,
    /// Source span of the key-label-set production.
    pub span: SourceSpan,
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
