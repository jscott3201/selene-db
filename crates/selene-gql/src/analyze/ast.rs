//! Analyzer output AST wrapper.

use selene_core::DbString;

use crate::{
    DdlStatement, GqlType, MutationPipeline, NonEmpty, ProcedureCall, QueryPipeline,
    SessionResetTarget, SessionSetGraphTarget, SetOp, SourceSpan, Statement, ValueExpr,
    analyze::{
        binding::BindingUse,
        category::StatementCategory,
        scope::{BindingScopeTree, ScopeId},
        types::{ExprIdLookup, ExprTypeTable},
        write_set::MutationWriteSet,
    },
};

/// A parsed and bind-pass-validated GQL statement.
#[derive(Clone, Debug)]
pub struct AnalyzedStatement {
    /// Original statement shape, preserved for planner input.
    pub statement: AnalyzedStatementKind,
    /// Full binding scope tree allocated during this analyze call.
    pub scopes: BindingScopeTree,
    /// Resolved binding references in source-walk order.
    pub references: Vec<BindingUse>,
    /// Inferred expression type cells.
    pub expr_types: ExprTypeTable,
    /// Expression-node to type-cell lookup for the owned statement AST.
    pub expr_ids: ExprIdLookup,
    /// Span of the root statement.
    pub span: SourceSpan,
    /// Per-statement classification for transaction-state enforcement.
    pub category: StatementCategory,
    /// Enumerated writes for mutation pipelines.
    pub write_set: Option<MutationWriteSet>,
}

impl AnalyzedStatement {
    pub(crate) fn new(
        statement: Statement,
        scopes: BindingScopeTree,
        references: Vec<BindingUse>,
        expr_types: ExprTypeTable,
        expr_ids: ExprIdLookup,
        category: StatementCategory,
        write_set: Option<MutationWriteSet>,
    ) -> Self {
        let span = statement.span();
        Self {
            statement: AnalyzedStatementKind::from_statement(statement),
            scopes,
            references,
            expr_types,
            expr_ids,
            span,
            category,
            write_set,
        }
    }

    /// Return the root statement scope.
    #[must_use]
    pub fn root_scope(&self) -> ScopeId {
        self.scopes.root()
    }
}

/// Top-level analyzed statement shape.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AnalyzedStatementKind {
    /// Read query pipeline.
    Query(QueryPipeline),
    /// Set-composed read pipelines.
    Composite {
        /// First pipeline.
        first: QueryPipeline,
        /// Remaining pipelines paired with their set operator.
        rest: NonEmpty<(SetOp, QueryPipeline)>,
        /// Source span.
        span: SourceSpan,
    },
    /// `NEXT`-chained read pipelines.
    Chained {
        /// Chained pipeline blocks.
        blocks: Vec<QueryPipeline>,
        /// Source span.
        span: SourceSpan,
    },
    /// Write-side mutation pipeline.
    Mutate(MutationPipeline),
    /// Data-definition statement.
    Ddl(DdlStatement),
    /// Top-level procedure call.
    Call(ProcedureCall),
    /// `EXPLAIN <statement>`.
    Explain {
        /// Inner analyzed statement shape.
        inner: Box<AnalyzedStatementKind>,
        /// Source span.
        span: SourceSpan,
    },
    /// `START TRANSACTION`.
    StartTransaction(SourceSpan),
    /// `COMMIT`.
    Commit(SourceSpan),
    /// `ROLLBACK`.
    Rollback(SourceSpan),
    /// `SESSION SET VALUE <param> [<type>] = <value expression>` (ISO feature GS03).
    SessionSetValue {
        /// Database-string parameter name without the leading `$`.
        param: DbString,
        /// Optional declared type for the target session parameter.
        declared_type: Option<GqlType>,
        /// Value expression bound to the parameter.
        value: Box<ValueExpr>,
        /// `IF NOT EXISTS` was present on the parameter specification.
        if_not_exists: bool,
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION SET TIME ZONE <time zone string>` (ISO feature GS15).
    SessionSetTimeZone {
        /// Decoded IANA region name or fixed-offset string.
        zone: String,
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION SET [PROPERTY] GRAPH <current graph>` (ISO/IEC 39075:2024 section 7.1).
    SessionSetGraph {
        /// Current-graph expression selected by the command.
        target: SessionSetGraphTarget,
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION RESET [ <session reset arguments> ]` (ISO features GS04/GS07/GS08/GS16).
    SessionReset {
        /// Reset target selected by the arguments.
        target: SessionResetTarget,
        /// Source span.
        span: SourceSpan,
    },
    /// `SESSION CLOSE` (ISO/IEC 39075:2024 section 7.3).
    SessionClose(SourceSpan),
}

impl AnalyzedStatementKind {
    fn from_statement(statement: Statement) -> Self {
        match statement {
            Statement::Query(value) => Self::Query(value),
            Statement::Composite { first, rest, span } => Self::Composite { first, rest, span },
            Statement::Chained { blocks, span } => Self::Chained { blocks, span },
            Statement::Mutate(value) => Self::Mutate(value),
            Statement::Ddl(value) => Self::Ddl(value),
            Statement::Call(value) => Self::Call(value),
            Statement::Explain { inner, span } => Self::Explain {
                inner: Box::new(Self::from_statement(*inner)),
                span,
            },
            Statement::StartTransaction { span } => Self::StartTransaction(span),
            Statement::Commit { span } => Self::Commit(span),
            Statement::Rollback { span } => Self::Rollback(span),
            Statement::SessionSetValue {
                param,
                declared_type,
                value,
                if_not_exists,
                span,
            } => Self::SessionSetValue {
                param,
                declared_type,
                value,
                if_not_exists,
                span,
            },
            Statement::SessionSetTimeZone { zone, span, .. } => {
                Self::SessionSetTimeZone { zone, span }
            }
            Statement::SessionSetGraph { target, span } => Self::SessionSetGraph { target, span },
            Statement::SessionReset { target, span } => Self::SessionReset { target, span },
            Statement::SessionClose { span } => Self::SessionClose(span),
        }
    }
}
