//! Analyzer diagnostics.

use selene_core::{IStr, LabelSet, PropertyValueType};

use crate::{
    BinaryOp, GqlStatus, GqlType, PathMode, PathSelector, ProcedureMutability, SourceSpan,
    analyze::binding::BindingDeclKind,
};

/// Semantic-analysis failure.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum AnalysisError {
    /// A reference does not resolve to any binding in the enclosing scopes.
    #[error("undefined reference: {name}")]
    #[diagnostic(code(SLENE_GQL_42N03))]
    UndefinedReference {
        /// Unresolved binding name.
        name: IStr,
        /// Source span of the unresolved reference.
        #[label("not bound in scope")]
        span: SourceSpan,
        /// Optional repair hint.
        #[help]
        hint: Option<String>,
    },

    /// A strict declaration site redeclared a binding already present in its scope.
    #[error("binding {name} is already declared in this scope")]
    #[diagnostic(code(SLENE_GQL_42N10))]
    Shadow {
        /// Redeclared binding name.
        name: IStr,
        /// Source span of the redeclaration.
        #[label("conflicts with an earlier binding")]
        span: SourceSpan,
        /// Source span of the prior declaration.
        #[label("first declared here")]
        prior_span: SourceSpan,
    },

    /// A pattern variable is reused with an element kind incompatible with
    /// its prior declaration (e.g. a node variable later used as an edge,
    /// or a path binding aliased over an existing node variable).
    #[error(
        "pattern variable {name} is already bound as a {prior} and cannot be reused as a {current}"
    )]
    #[diagnostic(code(SLENE_GQL_42N10))]
    PatternKindMismatch {
        /// Reused binding name.
        name: IStr,
        /// Element kind of the prior declaration.
        prior: PatternElementKind,
        /// Element kind of the new occurrence.
        current: PatternElementKind,
        /// Source span of the new occurrence.
        #[label("incompatible reuse")]
        span: SourceSpan,
        /// Source span of the prior declaration.
        #[label("first declared here")]
        prior_span: SourceSpan,
    },

    /// A value alias was reused where GQL requires a graph pattern binding.
    #[error(
        "binding {name} is already bound as {prior_kind:?} and cannot be reused as a {new_kind}"
    )]
    #[diagnostic(code(SLENE_GQL_42N10))]
    AliasReusedAsPatternBinding {
        /// Reused binding name.
        name: IStr,
        /// Prior non-pattern declaration kind.
        prior_kind: BindingDeclKind,
        /// New graph-pattern occurrence kind.
        new_kind: PatternElementKind,
        /// Source span of the new occurrence.
        #[label("alias cannot be reused as a pattern binding")]
        span: SourceSpan,
    },

    /// The analyzer encountered an AST surface it does not route yet.
    #[error("not implemented: {message}")]
    #[diagnostic(code(SLENE_GQL_42N01))]
    NotImplemented {
        /// Human-readable missing capability.
        message: String,
        /// Source span requiring the missing analyzer capability.
        #[label("not implemented yet")]
        span: SourceSpan,
        /// Optional implementation hint.
        #[help]
        hint: Option<String>,
    },

    /// ISO 16.4 forbids unbounded quantifiers without a restrictive or selective gate.
    #[error(
        "unbounded variable-length edge pattern requires a restrictive path mode, selective path selector, or DIFFERENT EDGES match mode"
    )]
    #[diagnostic(code(SLENE_GQL_42001))]
    UnboundedRequiresGate {
        /// Path mode in scope for the offending pattern.
        mode: PathMode,
        /// Path selector in scope for the offending pattern.
        selector: Option<PathSelector>,
        /// Source span of the unbounded quantifier.
        #[label("unbounded quantifier requires an ISO 16.4 gate")]
        span: SourceSpan,
    },

    /// ISO 20.6 scalar value query expression shape violation.
    #[error("invalid VALUE subquery shape: {message}")]
    #[diagnostic(code(SLENE_GQL_42001))]
    ValueSubqueryShapeViolation {
        /// Human-readable ISO 20.6 rule failure.
        message: String,
        /// Source span of the invalid VALUE subquery shape.
        #[label("violates ISO 20.6 scalar value query expression shape")]
        span: SourceSpan,
    },

    /// Analyzer expression recursion exceeded the implementation-defined cap.
    #[error("expression nesting depth {depth} exceeds analyzer limit")]
    #[diagnostic(code(SLENE_GQL_5GQL1))]
    RecursionLimitExceeded {
        /// Depth observed when the limit was exceeded.
        depth: u32,
    },

    /// A statically-decidable type mismatch.
    #[error("{context}: expected {expected}, found {found:?}")]
    #[diagnostic(code(SLENE_GQL_22G03))]
    TypeMismatch {
        /// Operation or clause that required a different type.
        context: TypeMismatchContext,
        /// Expected type category.
        expected: ExpectedType,
        /// Resolved type that violated the expectation.
        found: GqlType,
        /// Source span of the incompatible expression.
        #[label("incompatible type")]
        span: SourceSpan,
    },

    /// Procedure name was not registered.
    #[error("unknown procedure: {}", display_qualified_name(name))]
    #[diagnostic(code(SLENE_GQL_42N04))]
    UnknownProcedure {
        /// Qualified procedure name.
        name: Box<[IStr]>,
        /// Source span of the procedure call.
        #[label("procedure is not registered")]
        span: SourceSpan,
    },

    /// Procedure argument arity mismatch.
    #[error(
        "wrong argument count for {}: expected {}, found {actual}",
        display_qualified_name(procedure),
        display_argument_range(*minimum, *expected)
    )]
    #[diagnostic(code(SLENE_GQL_22G03))]
    WrongArgumentCount {
        /// Qualified procedure name.
        procedure: Box<[IStr]>,
        /// Maximum expected argument count.
        expected: usize,
        /// Minimum expected argument count.
        minimum: usize,
        /// Actual argument count.
        actual: usize,
        /// Source span of the procedure call.
        #[label("wrong number of arguments")]
        span: SourceSpan,
    },

    /// `YIELD col` referenced a column not in the procedure output schema.
    #[error(
        "unknown YIELD column {column} for procedure {}",
        display_qualified_name(procedure)
    )]
    #[diagnostic(code(SLENE_GQL_42N03))]
    UnknownYieldColumn {
        /// Qualified procedure name.
        procedure: Box<[IStr]>,
        /// Requested output column.
        column: IStr,
        /// Source span of the YIELD item.
        #[label("column is not produced by this procedure")]
        span: SourceSpan,
    },

    /// A read-only pipeline invoked a procedure declared as graph-writing,
    /// schema-writing, or administrative.
    #[error(
        "mutating procedure {} cannot be invoked in a read pipeline",
        display_qualified_name(procedure)
    )]
    #[diagnostic(code(SLENE_GQL_25G02))]
    MutatingProcedureInReadPipeline {
        /// Qualified procedure name.
        procedure: Box<[IStr]>,
        /// Declared procedure mutability.
        mutability: ProcedureMutability,
        /// Source span of the procedure call.
        #[label("read pipelines cannot invoke mutating procedures")]
        span: SourceSpan,
    },

    /// Static closed-graph validation found no matching node type.
    #[error("{labels:?} does not match any node type in graph type {graph_type}")]
    #[diagnostic(code(SLENE_A_010))]
    SchemaUnknownNodeType {
        /// Observed static label set.
        labels: LabelSet,
        /// Bound graph type name.
        graph_type: IStr,
        /// Source span of the offending label expression or pattern.
        #[label("unknown node type")]
        span: SourceSpan,
    },

    /// Static closed-graph validation found no matching edge type.
    #[error("edge label {label} does not match any edge type in graph type {graph_type}")]
    #[diagnostic(code(SLENE_A_011))]
    SchemaUnknownEdgeType {
        /// Edge label.
        label: IStr,
        /// Bound graph type name.
        graph_type: IStr,
        /// Source span of the offending edge label.
        #[label("unknown edge type")]
        span: SourceSpan,
    },

    /// Static closed-graph validation found an edge endpoint mismatch.
    #[error(
        "edge label {label}: declared as {expected_source} -> {expected_target} but used as {observed_source:?} -> {observed_target:?}"
    )]
    #[diagnostic(code(SLENE_A_012))]
    SchemaEdgeEndpointMismatch {
        /// Edge label.
        label: IStr,
        /// Expected source node type name.
        expected_source: String,
        /// Expected target node type name.
        expected_target: String,
        /// Observed source label set.
        observed_source: LabelSet,
        /// Observed target label set.
        observed_target: LabelSet,
        /// Source span of the offending edge pattern.
        #[label("endpoint types do not match edge declaration")]
        span: SourceSpan,
    },

    /// Static closed-graph validation found an undeclared property.
    #[error("property {property} is not declared by {declared_in} in graph type {graph_type}")]
    #[diagnostic(code(SLENE_A_013))]
    SchemaUndeclaredProperty {
        /// Undeclared property key.
        property: IStr,
        /// Node or edge type name that was checked.
        declared_in: IStr,
        /// Bound graph type name.
        graph_type: IStr,
        /// Source span of the property write.
        #[label("property is not declared")]
        span: SourceSpan,
    },

    /// Static closed-graph validation found a property value type mismatch.
    #[error("property {property} of {declared_in} declared {expected} but value is {found:?}")]
    #[diagnostic(code(SLENE_A_014))]
    SchemaPropertyTypeMismatch {
        /// Property key.
        property: IStr,
        /// Node or edge type name that declared the property.
        declared_in: IStr,
        /// Expected runtime storage type.
        expected: PropertyValueType,
        /// Statically inferred GQL type.
        found: GqlType,
        /// Source span of the offending value expression.
        #[label("value type is incompatible with property declaration")]
        span: SourceSpan,
    },

    /// Static closed-graph validation found a missing required property.
    #[error("required property {property} of {declared_in} missing at INSERT site")]
    #[diagnostic(code(SLENE_A_015))]
    SchemaRequiredPropertyMissing {
        /// Required property key.
        property: IStr,
        /// Node or edge type name that declared the property.
        declared_in: IStr,
        /// Source span of the insert pattern.
        #[label("required property is not supplied")]
        span: SourceSpan,
    },

    /// Static closed-graph validation found a required property removal.
    #[error("required property {property} of {declared_in} cannot be REMOVE'd")]
    #[diagnostic(code(SLENE_A_016))]
    SchemaRequiredPropertyRemoved {
        /// Required property key.
        property: IStr,
        /// Node or edge type name that declared the property.
        declared_in: IStr,
        /// Source span of the remove item.
        #[label("required property cannot be removed")]
        span: SourceSpan,
    },

    /// Static closed-graph validation found an invalid INSERT label expression.
    #[error("INSERT requires a single label or label conjunction; {form} is not allowed")]
    #[diagnostic(code(SLENE_A_017))]
    SchemaInvalidInsertLabelExpr {
        /// Invalid label-expression form.
        form: InvalidLabelForm,
        /// Source span of the invalid pattern.
        #[label("invalid INSERT label expression")]
        span: SourceSpan,
    },

    /// Static closed-graph validation found removal of an edge's required label.
    #[error("required edge label {label} of {declared_in} cannot be REMOVE'd")]
    #[diagnostic(code(SLENE_A_018))]
    SchemaRequiredEdgeLabelRemoved {
        /// Required edge label.
        label: IStr,
        /// Edge type name that declared the label.
        declared_in: IStr,
        /// Source span of the remove item.
        #[label("edge label cannot be removed")]
        span: SourceSpan,
    },
}

/// Label-expression forms that cannot identify a fresh closed-graph INSERT type.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InvalidLabelForm {
    /// Label disjunction, such as `:Person|Company`.
    Disjunction,
    /// Label negation, such as `:!Person`.
    Negation,
    /// Label wildcard, such as `:%`.
    Wildcard,
    /// No label expression was present.
    Missing,
}

impl std::fmt::Display for InvalidLabelForm {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Disjunction => "label disjunction",
            Self::Negation => "label negation",
            Self::Wildcard => "label wildcard",
            Self::Missing => "missing label",
        })
    }
}

/// Operation or clause that produced a type mismatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeMismatchContext {
    /// Binary arithmetic operator.
    BinaryArithmetic {
        /// Operator.
        op: BinaryOp,
        /// Offending operand side.
        side: Side,
    },
    /// Binary comparison operator.
    BinaryComparison {
        /// Operator.
        op: BinaryOp,
        /// Offending operand side.
        side: Side,
    },
    /// Binary boolean operator.
    BinaryBoolean {
        /// Operator.
        op: BinaryOp,
        /// Offending operand side.
        side: Side,
    },
    /// String/list concatenation.
    BinaryConcat {
        /// Offending operand side.
        side: Side,
    },
    /// String predicate operator.
    BinaryStringPredicate {
        /// Operator.
        op: BinaryOp,
        /// Offending operand side.
        side: Side,
    },
    /// LIKE predicate.
    LikePredicate {
        /// Offending operand side.
        side: Side,
    },
    /// Unary numeric negation.
    UnaryNegate,
    /// Unary boolean negation.
    UnaryNot,
    /// Unsupported `IS TYPED` target.
    IsTypedTarget,
    /// `IS NORMALIZED` operand.
    IsNormalized,
    /// CASE branch result unification failed.
    CaseBranchUnification,
    /// List literal element unification failed.
    ListLiteralUnification,
    /// IN-list value unification failed.
    InListUnification,
    /// BETWEEN operand or bound check failed.
    BetweenBounds {
        /// Offending operand side.
        side: Side,
    },
    /// Boolean condition clause check failed.
    Condition {
        /// Condition clause kind.
        clause: ConditionClause,
    },
    /// Procedure argument did not match the registered parameter type.
    ProcedureArgument {
        /// Qualified procedure name.
        procedure: Box<[IStr]>,
        /// Declared parameter name.
        parameter: IStr,
        /// Zero-based positional argument index.
        position: usize,
    },
}

impl std::fmt::Display for TypeMismatchContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinaryArithmetic { op, side } => {
                write!(f, "{side} operand of arithmetic operator {op:?}")
            }
            Self::BinaryComparison { op, side } => {
                write!(f, "{side} operand of comparison operator {op:?}")
            }
            Self::BinaryBoolean { op, side } => {
                write!(f, "{side} operand of boolean operator {op:?}")
            }
            Self::BinaryConcat { side } => write!(f, "{side} operand of concat operator"),
            Self::BinaryStringPredicate { op, side } => {
                write!(f, "{side} operand of string predicate {op:?}")
            }
            Self::LikePredicate { side } => write!(f, "{side} operand of LIKE predicate"),
            Self::UnaryNegate => f.write_str("operand of unary negate"),
            Self::UnaryNot => f.write_str("operand of unary NOT"),
            Self::IsTypedTarget => f.write_str("IS TYPED target"),
            Self::IsNormalized => f.write_str("IS NORMALIZED operand"),
            Self::CaseBranchUnification => f.write_str("CASE branch result"),
            Self::ListLiteralUnification => f.write_str("list literal element"),
            Self::InListUnification => f.write_str("IN-list value"),
            Self::BetweenBounds { side } => write!(f, "{side} operand of BETWEEN"),
            Self::Condition { clause } => write!(f, "{clause} condition"),
            Self::ProcedureArgument {
                procedure,
                parameter,
                position,
            } => {
                write!(f, "argument {position} ({parameter}) of ")?;
                fmt_qualified_name(f, procedure)
            }
        }
    }
}

fn display_qualified_name(name: &[IStr]) -> QualifiedNameDisplay<'_> {
    QualifiedNameDisplay(name)
}

struct QualifiedNameDisplay<'a>(&'a [IStr]);

impl std::fmt::Display for QualifiedNameDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt_qualified_name(f, self.0)
    }
}

fn fmt_qualified_name(f: &mut std::fmt::Formatter<'_>, name: &[IStr]) -> std::fmt::Result {
    let mut first = true;
    for segment in name {
        if !first {
            f.write_str(".")?;
        }
        let text = segment.as_str();
        if text.contains('.') || text.contains('"') {
            write!(f, "\"{}\"", text.replace('"', "\"\""))?;
        } else {
            f.write_str(text)?;
        }
        first = false;
    }
    Ok(())
}

fn display_argument_range(minimum: usize, maximum: usize) -> String {
    if minimum == maximum {
        maximum.to_string()
    } else {
        format!("{minimum}..={maximum}")
    }
}

/// Expected type category for a type mismatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpectedType {
    /// Any numeric type.
    Numeric,
    /// Boolean.
    Boolean,
    /// String.
    String,
    /// Comparable type.
    Comparable,
    /// List or string type.
    ListOrString,
    /// One specific GQL type.
    Specific(GqlType),
}

impl std::fmt::Display for ExpectedType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Numeric => f.write_str("numeric"),
            Self::Boolean => f.write_str("boolean"),
            Self::String => f.write_str("string"),
            Self::Comparable => f.write_str("comparable"),
            Self::ListOrString => f.write_str("list or string"),
            Self::Specific(ty) => write!(f, "{ty:?}"),
        }
    }
}

/// Operand side for binary type diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    /// Left-hand side.
    Lhs,
    /// Right-hand side.
    Rhs,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Lhs => "left",
            Self::Rhs => "right",
        })
    }
}

/// Boolean condition clause kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConditionClause {
    /// `MATCH ... WHERE`.
    MatchWhere,
    /// Node/edge pattern inline `WHERE`.
    InlineWhere,
    /// `FILTER`.
    Filter,
    /// `HAVING`.
    Having,
    /// `WITH ... WHERE`.
    WithWhere,
    /// `CASE WHEN`.
    CaseWhen,
}

impl std::fmt::Display for ConditionClause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MatchWhere => "MATCH WHERE",
            Self::InlineWhere => "inline WHERE",
            Self::Filter => "FILTER",
            Self::Having => "HAVING",
            Self::WithWhere => "WITH WHERE",
            Self::CaseWhen => "CASE WHEN",
        })
    }
}

/// Pattern element categories used by [`AnalysisError::PatternKindMismatch`].
///
/// The bind pass groups declaration sites by the graph element they introduce
/// (node, edge, path, value). Cross-category reuse via the same name is a
/// semantic error; same-category reuse is allowed (e.g., `MATCH (n)` followed
/// by `INSERT (n)-[:K]->(m)` legitimately reuses `n` as a node variable).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatternElementKind {
    /// `MATCH (n)` / `INSERT (n)`.
    Node,
    /// `MATCH ()-[e]->()` / `INSERT ()-[e]->()`.
    Edge,
    /// `path = (...)`.
    Path,
}

impl PatternElementKind {
    /// Categorize a [`BindingDeclKind`] for compatibility checks.
    #[must_use]
    pub const fn from_decl_kind(kind: BindingDeclKind) -> Option<Self> {
        match kind {
            BindingDeclKind::NodePattern | BindingDeclKind::InsertNode => Some(Self::Node),
            BindingDeclKind::EdgePattern | BindingDeclKind::InsertEdge => Some(Self::Edge),
            BindingDeclKind::PathBinding => Some(Self::Path),
            BindingDeclKind::LetAlias
            | BindingDeclKind::UnwindAlias
            | BindingDeclKind::ProjectionAlias
            | BindingDeclKind::YieldColumn => None,
        }
    }
}

impl std::fmt::Display for PatternElementKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Node => "node variable",
            Self::Edge => "edge variable",
            Self::Path => "path variable",
        })
    }
}

impl AnalysisError {
    /// Return this error's ISO GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> GqlStatus {
        match self {
            Self::UndefinedReference { .. } => GqlStatus::UNDEFINED_REFERENCE,
            Self::Shadow { .. }
            | Self::PatternKindMismatch { .. }
            | Self::AliasReusedAsPatternBinding { .. } => GqlStatus::DUPLICATE_OBJECT,
            Self::NotImplemented { .. } => GqlStatus::FEATURE_NOT_SUPPORTED,
            Self::UnboundedRequiresGate { .. } => GqlStatus::SYNTAX_ERROR,
            Self::ValueSubqueryShapeViolation { .. } => GqlStatus::SYNTAX_ERROR,
            Self::RecursionLimitExceeded { .. } => GqlStatus::PROGRAM_LIMIT_EXCEEDED,
            Self::TypeMismatch { .. } => GqlStatus::DATATYPE_MISMATCH,
            Self::UnknownProcedure { .. } => GqlStatus::UNKNOWN_PROCEDURE,
            Self::WrongArgumentCount { .. } => GqlStatus::DATATYPE_MISMATCH,
            Self::UnknownYieldColumn { .. } => GqlStatus::UNDEFINED_REFERENCE,
            Self::MutatingProcedureInReadPipeline { .. } => {
                GqlStatus::INVALID_TRANSACTION_STATE_MIXING
            }
            Self::SchemaUnknownNodeType { .. }
            | Self::SchemaUnknownEdgeType { .. }
            | Self::SchemaEdgeEndpointMismatch { .. }
            | Self::SchemaUndeclaredProperty { .. }
            | Self::SchemaPropertyTypeMismatch { .. }
            | Self::SchemaRequiredPropertyMissing { .. }
            | Self::SchemaRequiredPropertyRemoved { .. }
            | Self::SchemaInvalidInsertLabelExpr { .. }
            | Self::SchemaRequiredEdgeLabelRemoved { .. } => GqlStatus::GRAPH_TYPE_VIOLATION,
        }
    }

    pub(crate) fn undefined_reference(name: IStr, span: SourceSpan) -> Self {
        Self::UndefinedReference {
            name,
            span,
            hint: Some("declare the variable before this reference".into()),
        }
    }
}
