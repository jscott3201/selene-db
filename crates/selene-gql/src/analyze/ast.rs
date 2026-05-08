//! Analyzer output AST wrapper.

use crate::{
    DdlStatement, MutationPipeline, ProcedureCall, QueryPipeline, SetOp, SourceSpan, Statement,
    analyze::{
        binding::BindingUse,
        scope::{BindingScopeTree, ScopeId},
        types::{ExprIdMap, ExprTypeTable},
    },
};

/// A parsed and bind-pass-validated GQL statement.
#[derive(Debug)]
pub struct AnalyzedStatement {
    /// Original statement shape, preserved for planner input.
    pub statement: AnalyzedStatementKind,
    /// Full binding scope tree allocated during this analyze call.
    pub scopes: BindingScopeTree,
    /// Resolved binding references in source-walk order.
    pub references: Vec<BindingUse>,
    /// `CALL ... YIELD *` wildcard occurrences awaiting BRIEF-23 expansion.
    pub yield_stars: Vec<SourceSpan>,
    /// Inferred expression type cells.
    pub expr_types: ExprTypeTable,
    /// Expression-node to type-cell lookup for the owned statement AST.
    pub expr_ids: ExprIdMap,
    /// Span of the root statement.
    pub span: SourceSpan,
}

impl AnalyzedStatement {
    pub(crate) fn new(
        statement: Statement,
        scopes: BindingScopeTree,
        references: Vec<BindingUse>,
        yield_stars: Vec<SourceSpan>,
        expr_types: ExprTypeTable,
        expr_ids: ExprIdMap,
    ) -> Self {
        let span = statement.span();
        Self {
            statement: AnalyzedStatementKind::from_statement(statement),
            scopes,
            references,
            yield_stars,
            expr_types,
            expr_ids,
            span,
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
        rest: Vec<(SetOp, QueryPipeline)>,
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
    /// `START TRANSACTION`.
    StartTransaction(SourceSpan),
    /// `COMMIT`.
    Commit(SourceSpan),
    /// `ROLLBACK`.
    Rollback(SourceSpan),
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
            Statement::StartTransaction { span } => Self::StartTransaction(span),
            Statement::Commit { span } => Self::Commit(span),
            Statement::Rollback { span } => Self::Rollback(span),
        }
    }
}
