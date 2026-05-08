//! Analyzer diagnostics.

use selene_core::IStr;

use crate::{BinaryOp, GqlStatus, GqlType, SourceSpan, analyze::binding::BindingDeclKind};

/// Semantic-analysis failure.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum AnalysisError {
    /// A reference does not resolve to any binding in the enclosing scopes.
    #[error("undefined reference: {name}")]
    #[diagnostic(code(SLENE_GQL_42703))]
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
    #[diagnostic(code(SLENE_GQL_42710))]
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
    #[diagnostic(code(SLENE_GQL_42710))]
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

    /// The analyzer encountered an AST surface it does not route yet.
    #[error("not implemented: {message}")]
    #[diagnostic(code(SLENE_GQL_0A000))]
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

    /// A statically-decidable type mismatch.
    #[error("{context}: expected {expected}, found {found:?}")]
    #[diagnostic(code(SLENE_GQL_42883))]
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
}

/// Operation or clause that produced a type mismatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
        }
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
            Self::Shadow { .. } | Self::PatternKindMismatch { .. } => GqlStatus::DUPLICATE_OBJECT,
            Self::NotImplemented { .. } => GqlStatus::FEATURE_NOT_SUPPORTED,
            Self::TypeMismatch { .. } => GqlStatus::DATATYPE_MISMATCH,
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
