//! Expression AST nodes.

use std::sync::Arc;

use rust_decimal::Decimal;
use selene_core::DbString;

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
        /// Database-string variable name.
        name: DbString,
        /// Source span of the variable reference.
        span: SourceSpan,
    },
    /// Query parameter reference, such as `$name`.
    Parameter {
        /// Database-string parameter name without the leading `$`.
        name: DbString,
        /// Optional inline declared parameter type.
        declared_type: Option<GqlType>,
        /// Source span of the parameter reference.
        span: SourceSpan,
    },
    /// Property access, such as `n.name`.
    PropertyAccess {
        /// Target expression.
        target: Box<ValueExpr>,
        /// Database-string property key.
        key: DbString,
        /// Source span of the full property access.
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
        fields: Vec<(DbString, ValueExpr)>,
        /// Source span of the record.
        span: SourceSpan,
    },
    /// `PATH[<node>, <edge>, <node>, ...]` constructor.
    PathConstructor {
        /// Alternating node/edge/node reference expressions.
        elements: Vec<ValueExpr>,
        /// Source span of the path constructor.
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
        /// Qualified function name as a list of database-string segments.
        ///
        /// Stored as a path so that `foo."bar.baz"` and `foo.bar.baz`
        /// (which a flat string would alias) parse to distinguishable
        /// values. Bare functions (`count(...)`) are a single-element
        /// path; namespaced calls (`db.bar()`, `pkg.subpkg.fn()`) preserve
        /// every segment.
        name: NonEmpty<DbString>,
        /// Function arguments.
        args: Vec<ValueExpr>,
        /// `true` for `count(*)`.
        star: bool,
        /// `true` when the call included `DISTINCT`.
        distinct: bool,
        /// Source span of the call.
        span: SourceSpan,
    },
    /// `DURATION_BETWEEN(<start>, <end>) [<temporal duration qualifier>]`.
    DurationBetween {
        /// Start temporal value expression.
        start: Box<ValueExpr>,
        /// End temporal value expression.
        end: Box<ValueExpr>,
        /// Requested duration unit group.
        qualifier: TemporalDurationQualifier,
        /// Source span of the full expression.
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
    /// `[NOT] IN` predicate with a list-valued expression on the right.
    InListExpression {
        /// Checked operand.
        operand: Box<ValueExpr>,
        /// List-valued expression.
        list: Box<ValueExpr>,
        /// Whether the predicate was negated.
        negated: bool,
        /// Source span of the full predicate.
        span: SourceSpan,
    },
    /// `ALL_DIFFERENT(...)` predicate.
    ///
    /// ISO/IEC 39075:2024 section 19.11 takes element variable references.
    /// The AST keeps a broad item shape so parser construction stays regular;
    /// analysis enforces the ISO argument rule.
    AllDifferent {
        /// Items to compare.
        items: Vec<ValueExpr>,
        /// Source span of the predicate.
        span: SourceSpan,
    },
    /// `SAME(...)` predicate.
    ///
    /// ISO/IEC 39075:2024 section 19.12 takes element variable references.
    /// The AST keeps a broad item shape so parser construction stays regular;
    /// analysis enforces the ISO argument rule.
    Same {
        /// Items to compare.
        items: Vec<ValueExpr>,
        /// Source span of the predicate.
        span: SourceSpan,
    },
    /// `PROPERTY_EXISTS(target, 'key')` predicate.
    ///
    /// ISO/IEC 39075:2024 section 19.13 takes an element variable reference
    /// and property name. The AST keeps a broad target shape so parser
    /// construction stays regular; analysis enforces the ISO target rule.
    PropertyExists {
        /// Target expression.
        target: Box<ValueExpr>,
        /// Database-string property key.
        key: DbString,
        /// Source spelling class for the property-key character string literal.
        key_source_kind: CharacterStringLiteralKind,
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
    /// `EXISTS { ... }` subquery predicate.
    Exists {
        /// Nested body.
        body: ExistsBody,
        /// Whether the predicate was negated.
        negated: bool,
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

/// Nested body accepted by an `EXISTS` predicate.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum ExistsBody {
    /// Existing single-MATCH body.
    Match(Box<MatchClause>),
    /// Full query pipeline body.
    Query(Box<QueryPipeline>),
}

/// Temporal duration qualifier for `DURATION_BETWEEN`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum TemporalDurationQualifier {
    /// `YEAR TO MONTH`.
    YearToMonth,
    /// `DAY TO SECOND`.
    DayToSecond,
}

impl Drop for ValueExpr {
    /// Tear this expression down iteratively rather than with the compiler's
    /// derived recursive destructor.
    ///
    /// A left-leaning `Box<ValueExpr>` chain (e.g. the depth-N tree built from
    /// `a OR a OR …` or a long `a.b.c.…` access chain) overflows the native
    /// stack when freed recursively — one frame per level — at roughly 130k
    /// deep. A Rust stack overflow is **non-unwindable** (`catch_unwind` cannot
    /// trap it), so it hard-kills the host process embedding selene-db. The
    /// parser caps *accepted* expression depth far below that
    /// (`parser::depth`), but an over-cap tree is still fully constructed before
    /// the cap rejects it, and that rejected tree must be dropped without
    /// recursing. This manual `Drop` hoists every owned child `ValueExpr` onto a
    /// heap worklist (replacing it in place with a childless placeholder) and
    /// drops the worklist in a pop-loop, so teardown is O(nodes) iterative.
    ///
    /// By the time any hoisted node is itself dropped, its children have already
    /// been replaced by placeholders, so its re-entrant destructor finds nothing
    /// to recurse into (recursion depth stays ≤ 2 regardless of tree depth).
    /// Subquery bodies (`Box<MatchClause>` / `Box<QueryPipeline>`) and the
    /// `Cast` target type (`Box<GqlType>`) are **not** direct `ValueExpr`
    /// children (see [`ValueExpr::for_each_child_mut`]); they drop normally,
    /// which is safe because subquery nesting is bounded by the pre-pest `{`
    /// delimiter cap and `GqlType` depth by the type builder's gate.
    fn drop(&mut self) {
        let mut pending: Vec<ValueExpr> = Vec::new();
        hoist_children(self, &mut pending);
        while let Some(mut node) = pending.pop() {
            hoist_children(&mut node, &mut pending);
            // `node` drops here with its children already hoisted out.
        }
    }
}

/// Move every direct child [`ValueExpr`] of `expr` onto `pending`, leaving a
/// childless placeholder in its slot. Shared by the iterative [`Drop`] impl.
fn hoist_children(expr: &mut ValueExpr, pending: &mut Vec<ValueExpr>) {
    expr.for_each_child_mut(&mut |child| {
        pending.push(core::mem::replace(child, drop_placeholder()));
    });
}

/// A cheap, childless [`ValueExpr`] swapped in for a hoisted child during
/// iterative teardown. Constructing it allocates nothing and dropping it
/// recurses into nothing.
fn drop_placeholder() -> ValueExpr {
    ValueExpr::Literal(Literal::Null(SourceSpan::default()))
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
            | Self::ListLiteral { span, .. }
            | Self::RecordLiteral { span, .. }
            | Self::PathConstructor { span, .. }
            | Self::BinaryOp { span, .. }
            | Self::UnaryOp { span, .. }
            | Self::FunctionCall { span, .. }
            | Self::DurationBetween { span, .. }
            | Self::IsCheck { span, .. }
            | Self::InList { span, .. }
            | Self::InListExpression { span, .. }
            | Self::AllDifferent { span, .. }
            | Self::Same { span, .. }
            | Self::PropertyExists { span, .. }
            | Self::Case { span, .. }
            | Self::Exists { span, .. }
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
    /// String/list/bytes/path concatenation.
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
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum Literal {
    /// Boolean literal.
    Bool(bool, SourceSpan),
    /// Signed 64-bit integer literal.
    Integer(i64, SourceSpan),
    /// Signed 64-bit integer literal written with a non-decimal radix prefix.
    RadixInteger(i64, SourceSpan, IntegerLiteralKind),
    /// Exact fixed-precision decimal literal.
    Decimal(Decimal, SourceSpan, DecimalLiteralKind),
    /// 64-bit floating-point literal.
    Float(f64, SourceSpan, FloatLiteralKind),
    /// Database-string literal.
    String(DbString, SourceSpan, CharacterStringLiteralKind),
    /// Byte-string literal.
    Bytes(Arc<[u8]>, SourceSpan),
    /// UUID literal.
    Uuid(uuid::Uuid, SourceSpan, CharacterStringLiteralKind),
    /// Zoned datetime literal.
    ZonedDateTime(Box<jiff::Zoned>, SourceSpan, CharacterStringLiteralKind),
    /// Local datetime literal.
    LocalDateTime(
        jiff::civil::DateTime,
        SourceSpan,
        CharacterStringLiteralKind,
    ),
    /// Date literal.
    Date(jiff::civil::Date, SourceSpan, CharacterStringLiteralKind),
    /// Zoned time literal.
    ZonedTime(Box<jiff::Zoned>, SourceSpan, CharacterStringLiteralKind),
    /// Local time literal.
    LocalTime(jiff::civil::Time, SourceSpan, CharacterStringLiteralKind),
    /// Duration literal.
    Duration(Box<jiff::Span>, SourceSpan, CharacterStringLiteralKind),
    /// Null literal.
    Null(SourceSpan),
}

/// Source spelling class for an ISO character string literal.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum CharacterStringLiteralKind {
    /// Standard character string literal with character escaping enabled.
    Escaped,
    /// Character string literal prefixed by `<no escape>` (`@`), disabling
    /// backslash and doubled-delimiter character representations.
    NoEscape,
}

/// Source spelling class for an integer literal.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum IntegerLiteralKind {
    /// Hexadecimal literal with a `0x` prefix.
    Hexadecimal,
    /// Octal literal with a `0o` prefix.
    Octal,
    /// Binary literal with a `0b` prefix.
    Binary,
}

/// Source spelling class for an exact decimal literal.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum DecimalLiteralKind {
    /// Common decimal notation without an exact-number suffix.
    CommonWithoutSuffix,
    /// Common decimal notation or decimal integer with an exact-number suffix.
    CommonOrIntegerWithSuffix,
    /// Scientific decimal notation with an exact-number suffix.
    ScientificWithSuffix,
}

/// Source spelling class for an approximate numeric literal.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum FloatLiteralKind {
    /// Scientific decimal notation without an approximate-number suffix.
    ScientificWithoutSuffix,
    /// Common decimal notation or decimal integer with an `F` suffix.
    CommonOrIntegerWithFloatSuffix,
    /// Common decimal notation or decimal integer with a `D` suffix.
    CommonOrIntegerWithDoubleSuffix,
    /// Scientific decimal notation with an `F` suffix.
    ScientificWithFloatSuffix,
    /// Scientific decimal notation with a `D` suffix.
    ScientificWithDoubleSuffix,
}

impl PartialEq for Literal {
    fn eq(&self, rhs: &Self) -> bool {
        match (self, rhs) {
            (Self::Bool(lhs, lhs_span), Self::Bool(rhs, rhs_span)) => {
                lhs == rhs && lhs_span == rhs_span
            }
            (Self::Integer(lhs, lhs_span), Self::Integer(rhs, rhs_span)) => {
                lhs == rhs && lhs_span == rhs_span
            }
            (
                Self::RadixInteger(lhs, lhs_span, lhs_kind),
                Self::RadixInteger(rhs, rhs_span, rhs_kind),
            ) => lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind,
            (Self::Decimal(lhs, lhs_span, lhs_kind), Self::Decimal(rhs, rhs_span, rhs_kind)) => {
                lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind
            }
            (Self::Float(lhs, lhs_span, lhs_kind), Self::Float(rhs, rhs_span, rhs_kind)) => {
                lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind
            }
            (Self::String(lhs, lhs_span, lhs_kind), Self::String(rhs, rhs_span, rhs_kind)) => {
                lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind
            }
            (Self::Bytes(lhs, lhs_span), Self::Bytes(rhs, rhs_span)) => {
                lhs == rhs && lhs_span == rhs_span
            }
            (Self::Uuid(lhs, lhs_span, lhs_kind), Self::Uuid(rhs, rhs_span, rhs_kind)) => {
                lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind
            }
            (
                Self::ZonedDateTime(lhs, lhs_span, lhs_kind),
                Self::ZonedDateTime(rhs, rhs_span, rhs_kind),
            ) => lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind,
            (
                Self::LocalDateTime(lhs, lhs_span, lhs_kind),
                Self::LocalDateTime(rhs, rhs_span, rhs_kind),
            ) => lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind,
            (Self::Date(lhs, lhs_span, lhs_kind), Self::Date(rhs, rhs_span, rhs_kind)) => {
                lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind
            }
            (
                Self::ZonedTime(lhs, lhs_span, lhs_kind),
                Self::ZonedTime(rhs, rhs_span, rhs_kind),
            ) => lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind,
            (
                Self::LocalTime(lhs, lhs_span, lhs_kind),
                Self::LocalTime(rhs, rhs_span, rhs_kind),
            ) => lhs == rhs && lhs_span == rhs_span && lhs_kind == rhs_kind,
            (Self::Duration(lhs, lhs_span, lhs_kind), Self::Duration(rhs, rhs_span, rhs_kind)) => {
                lhs.fieldwise() == rhs.fieldwise() && lhs_span == rhs_span && lhs_kind == rhs_kind
            }
            (Self::Null(lhs_span), Self::Null(rhs_span)) => lhs_span == rhs_span,
            _ => false,
        }
    }
}

impl Literal {
    /// Return the source span for this literal.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Bool(_, span)
            | Self::Integer(_, span)
            | Self::RadixInteger(_, span, _)
            | Self::Decimal(_, span, _)
            | Self::Float(_, span, _)
            | Self::String(_, span, _)
            | Self::Bytes(_, span)
            | Self::Uuid(_, span, _)
            | Self::ZonedDateTime(_, span, _)
            | Self::LocalDateTime(_, span, _)
            | Self::Date(_, span, _)
            | Self::ZonedTime(_, span, _)
            | Self::LocalTime(_, span, _)
            | Self::Duration(_, span, _)
            | Self::Null(span) => *span,
        }
    }

    /// Return a mutable reference to this literal's source span.
    ///
    /// Used by [`ValueExpr::for_each_span_mut`](crate::ValueExpr::for_each_span_mut)
    /// so span-rewriting walkers (parser span rebasing, span-erasing equality)
    /// can reach the span stored inside the literal variant.
    pub const fn span_mut(&mut self) -> &mut SourceSpan {
        match self {
            Self::Bool(_, span)
            | Self::Integer(_, span)
            | Self::RadixInteger(_, span, _)
            | Self::Decimal(_, span, _)
            | Self::Float(_, span, _)
            | Self::String(_, span, _)
            | Self::Bytes(_, span)
            | Self::Uuid(_, span, _)
            | Self::ZonedDateTime(_, span, _)
            | Self::LocalDateTime(_, span, _)
            | Self::Date(_, span, _)
            | Self::ZonedTime(_, span, _)
            | Self::LocalTime(_, span, _)
            | Self::Duration(_, span, _)
            | Self::Null(span) => span,
        }
    }
}
