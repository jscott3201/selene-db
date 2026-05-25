//! Expression AST nodes.

use selene_core::IStr;

use crate::ast::{
    pattern::LabelExpr, pattern::MatchClause, span::SourceSpan, statement::QueryPipeline,
    types::GqlType, util::NonEmpty,
};

/// Value expression.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum ValueExpr {
    /// Literal value expression.
    Literal(Literal),
    /// Variable reference parsed from an identifier token.
    Variable {
        /// Interned variable name.
        name: IStr,
        /// Source span of the variable reference.
        span: SourceSpan,
    },
    /// Query parameter reference, such as `$name`.
    Parameter {
        /// Interned parameter name without the leading `$`.
        name: IStr,
        /// Optional inline declared parameter type.
        declared_type: Option<GqlType>,
        /// Source span of the parameter reference.
        span: SourceSpan,
    },
    /// Property access, such as `n.name`.
    PropertyAccess {
        /// Target expression.
        target: Box<ValueExpr>,
        /// Interned property key.
        key: IStr,
        /// Source span of the full property access.
        span: SourceSpan,
    },
    /// List element access, such as `items[0]`.
    ListAccess {
        /// Target list expression.
        target: Box<ValueExpr>,
        /// Index expression.
        index: Box<ValueExpr>,
        /// Source span of the full list access.
        span: SourceSpan,
    },
    /// List literal.
    ListLiteral {
        /// Literal items.
        items: Vec<ValueExpr>,
        /// Source span of the list.
        span: SourceSpan,
    },
    /// Record literal.
    RecordLiteral {
        /// Record fields in source order.
        fields: Vec<(IStr, ValueExpr)>,
        /// Source span of the record.
        span: SourceSpan,
    },
    /// Binary operator expression.
    BinaryOp {
        /// Operator.
        op: BinaryOp,
        /// Left operand.
        lhs: Box<ValueExpr>,
        /// Right operand.
        rhs: Box<ValueExpr>,
        /// Source span of the full expression.
        span: SourceSpan,
    },
    /// Unary operator expression.
    UnaryOp {
        /// Operator.
        op: UnaryOp,
        /// Operand.
        operand: Box<ValueExpr>,
        /// Source span of the full expression.
        span: SourceSpan,
    },
    /// Function call, including aggregate-looking calls.
    FunctionCall {
        /// Qualified function name as a list of interned segments.
        ///
        /// Stored as a path so that `foo."bar.baz"` and `foo.bar.baz`
        /// (which a flat string would alias) parse to distinguishable
        /// values. Bare functions (`count(...)`) are a single-element
        /// path; namespaced calls (`db.bar()`, `pkg.subpkg.fn()`) preserve
        /// every segment.
        name: NonEmpty<IStr>,
        /// Function arguments.
        args: Vec<ValueExpr>,
        /// `true` for `count(*)`.
        star: bool,
        /// `true` when the call included `DISTINCT`.
        distinct: bool,
        /// Source span of the call.
        span: SourceSpan,
    },
    /// `IS` predicate family.
    IsCheck {
        /// Checked operand.
        operand: Box<ValueExpr>,
        /// Predicate kind.
        kind: IsCheckKind,
        /// Whether the predicate was negated.
        negated: bool,
        /// Source span of the full predicate.
        span: SourceSpan,
    },
    /// `[NOT] IN` predicate.
    InList {
        /// Checked operand.
        operand: Box<ValueExpr>,
        /// List values.
        list: Vec<ValueExpr>,
        /// Whether the predicate was negated.
        negated: bool,
        /// Source span of the full predicate.
        span: SourceSpan,
    },
    /// `[NOT] LIKE` predicate.
    ///
    /// selene-db extension: not in ISO/IEC 39075:2024 section 19; modeled
    /// after SQL `LIKE`.
    Like {
        /// Checked operand.
        operand: Box<ValueExpr>,
        /// Pattern expression.
        pattern: Box<ValueExpr>,
        /// Whether the predicate was negated.
        negated: bool,
        /// Source span of the full predicate.
        span: SourceSpan,
    },
    /// `[NOT] BETWEEN` predicate.
    ///
    /// selene-db extension: not in ISO/IEC 39075:2024 section 19; modeled
    /// after SQL `BETWEEN`.
    Between {
        /// Checked operand.
        operand: Box<ValueExpr>,
        /// Lower bound.
        low: Box<ValueExpr>,
        /// Upper bound.
        high: Box<ValueExpr>,
        /// Whether the predicate was negated.
        negated: bool,
        /// Source span of the full predicate.
        span: SourceSpan,
    },
    /// `ALL_DIFFERENT(...)` predicate.
    ///
    /// selene-db extension: ISO/IEC 39075:2024 section 19.11 takes element
    /// variable references; this AST keeps the broader expression shape.
    AllDifferent {
        /// Items to compare.
        items: Vec<ValueExpr>,
        /// Source span of the predicate.
        span: SourceSpan,
    },
    /// `SAME(...)` predicate.
    ///
    /// selene-db extension: ISO/IEC 39075:2024 section 19.12 takes element
    /// variable references; this AST keeps the broader expression shape.
    Same {
        /// Items to compare.
        items: Vec<ValueExpr>,
        /// Source span of the predicate.
        span: SourceSpan,
    },
    /// `PROPERTY_EXISTS(target, 'key')` predicate.
    ///
    /// selene-db extension: ISO/IEC 39075:2024 section 19.13 takes an element
    /// variable reference and property name; this AST accepts any target expression.
    PropertyExists {
        /// Target expression.
        target: Box<ValueExpr>,
        /// Interned property key.
        key: IStr,
        /// Source span of the predicate.
        span: SourceSpan,
    },
    /// `CASE` expression.
    Case {
        /// `(condition, value)` branches.
        branches: Vec<(ValueExpr, ValueExpr)>,
        /// Optional else branch.
        else_branch: Option<Box<ValueExpr>>,
        /// Source span of the full expression.
        span: SourceSpan,
    },
    /// `EXISTS { MATCH ... }` subquery predicate.
    Exists {
        /// Match pattern.
        pattern: Box<MatchClause>,
        /// Whether the predicate was negated.
        negated: bool,
        /// Source span of the full expression.
        span: SourceSpan,
    },
    /// `COUNT { MATCH ... }` subquery expression.
    CountSubquery {
        /// Match pattern.
        pattern: Box<MatchClause>,
        /// Source span of the full expression.
        span: SourceSpan,
    },
    /// `VALUE { ... }` scalar value query expression.
    ValueSubquery {
        /// Nested query body.
        body: Box<QueryPipeline>,
        /// Source span of the full expression.
        span: SourceSpan,
    },
    /// `NORMALIZE(<source>[, <normal form>])` Unicode normalization expression.
    Normalize {
        /// Source string expression.
        source: Box<ValueExpr>,
        /// Optional normalization form; omitted form defaults to NFC.
        form: Option<NormalForm>,
        /// Source span of the full expression.
        span: SourceSpan,
    },
    /// Explicit `TRIM([LEADING|TRAILING|BOTH] [char] FROM source)` expression.
    Trim {
        /// Trim direction.
        spec: TrimSpec,
        /// Optional trim character expression; omitted character defaults to space.
        character: Option<Box<ValueExpr>>,
        /// Source string expression.
        source: Box<ValueExpr>,
        /// Source span of the full expression.
        span: SourceSpan,
    },
    /// `CAST(<value> AS <target_type>)` explicit cast expression per ISO §22.
    ///
    /// `target_type` is boxed to keep the size of [`ValueExpr`] bounded; the
    /// nested `GqlType::List(Box<GqlType>)` chain is otherwise heap-resident
    /// already, and the analyzer-recursion test (`addition_statement(100)`)
    /// stack-overflows if a non-boxed `GqlType` is added inline.
    Cast {
        /// Source expression being cast.
        value: Box<ValueExpr>,
        /// Declared target GQL type.
        target_type: Box<GqlType>,
        /// Source span of the full expression.
        span: SourceSpan,
    },
}

impl ValueExpr {
    /// Return the source span for this expression.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Literal(literal) => literal.span(),
            Self::Variable { span, .. }
            | Self::Parameter { span, .. }
            | Self::PropertyAccess { span, .. }
            | Self::ListAccess { span, .. }
            | Self::ListLiteral { span, .. }
            | Self::RecordLiteral { span, .. }
            | Self::BinaryOp { span, .. }
            | Self::UnaryOp { span, .. }
            | Self::FunctionCall { span, .. }
            | Self::IsCheck { span, .. }
            | Self::InList { span, .. }
            | Self::Like { span, .. }
            | Self::Between { span, .. }
            | Self::AllDifferent { span, .. }
            | Self::Same { span, .. }
            | Self::PropertyExists { span, .. }
            | Self::Case { span, .. }
            | Self::Exists { span, .. }
            | Self::CountSubquery { span, .. }
            | Self::ValueSubquery { span, .. }
            | Self::Normalize { span, .. }
            | Self::Trim { span, .. }
            | Self::Cast { span, .. } => *span,
        }
    }
}

/// Binary operator.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum BinaryOp {
    /// Addition.
    Add,
    /// Subtraction.
    Sub,
    /// Multiplication.
    Mul,
    /// Division.
    Div,
    /// Remainder.
    Mod,
    /// Exponentiation.
    ///
    /// selene-db extension: ISO/IEC 39075:2024 section 20.22 uses `POWER(x, y)`;
    /// the grammar does not emit `^` today.
    Power,
    /// Equality.
    Eq,
    /// Inequality.
    Ne,
    /// Less-than.
    Lt,
    /// Less-than-or-equal.
    Le,
    /// Greater-than.
    Gt,
    /// Greater-than-or-equal.
    Ge,
    /// Boolean conjunction.
    And,
    /// Boolean disjunction.
    Or,
    /// Boolean exclusive-or.
    Xor,
    /// String/list concatenation.
    Concat,
    /// `CONTAINS`.
    Contains,
    /// `STARTS WITH`.
    StartsWith,
    /// `ENDS WITH`.
    EndsWith,
}

/// Unary operator.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum UnaryOp {
    /// Numeric negation.
    Negate,
    /// Boolean negation.
    Not,
}

/// `IS` predicate kind.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum IsCheckKind {
    /// `IS NULL`.
    Null,
    /// `IS DIRECTED`.
    Directed,
    /// `IS LABELED`.
    Labeled(LabelExpr),
    /// `IS TRUE/FALSE/UNKNOWN`.
    TruthValue(TruthValue),
    /// `IS TYPED`.
    Typed(GqlType),
    /// `IS NORMALIZED`.
    Normalized(NormalForm),
    /// `IS SOURCE OF`.
    SourceOf(Box<ValueExpr>),
    /// `IS DESTINATION OF`.
    DestinationOf(Box<ValueExpr>),
}

/// Three-valued boolean predicate target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum TruthValue {
    /// `TRUE`.
    True,
    /// `FALSE`.
    False,
    /// `UNKNOWN`.
    Unknown,
}

/// Unicode normalization form.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum NormalForm {
    /// NFC.
    Nfc,
    /// NFD.
    Nfd,
    /// NFKC.
    Nfkc,
    /// NFKD.
    Nfkd,
}

/// Explicit TRIM direction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum TrimSpec {
    /// Trim leading characters.
    Leading,
    /// Trim trailing characters.
    Trailing,
    /// Trim both leading and trailing characters.
    Both,
}

/// Literal expression.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum Literal {
    /// Boolean literal.
    Bool(bool, SourceSpan),
    /// Signed 64-bit integer literal.
    Integer(i64, SourceSpan),
    /// 64-bit floating-point literal.
    Float(f64, SourceSpan),
    /// Interned string literal.
    String(IStr, SourceSpan),
    /// UUID literal.
    Uuid(uuid::Uuid, SourceSpan),
    /// Null literal.
    Null(SourceSpan),
}

impl Literal {
    /// Return the source span for this literal.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Bool(_, span)
            | Self::Integer(_, span)
            | Self::Float(_, span)
            | Self::String(_, span)
            | Self::Uuid(_, span)
            | Self::Null(span) => *span,
        }
    }
}
