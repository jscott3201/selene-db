//! Bind-pass orchestration.

pub(crate) mod call;
pub(crate) mod ddl;
pub(crate) mod expr;
pub(crate) mod mutation;
pub(crate) mod pattern;
pub(crate) mod query;
pub(crate) mod transaction;

use selene_core::IStr;

use crate::{
    SourceSpan, Statement,
    analyze::{
        ast::AnalyzedStatement,
        binding::{BindingDeclKind, BindingId, BindingUse, BindingUseKind},
        error::AnalysisError,
        scope::{BindingScopeTree, ScopeId, ScopeKind},
    },
};

/// Analyze one statement with the BRIEF-21 binding pass.
pub(crate) fn bind_statement(stmt: Statement) -> Result<AnalyzedStatement, AnalysisError> {
    let mut ctx = BindContext::new(stmt.span());
    match &stmt {
        Statement::Query(pipeline) => query::bind_query_pipeline(&mut ctx, pipeline)?,
        Statement::Composite { first, rest, .. } => {
            ctx.with_child_scope(ScopeKind::Projection, first.span, true, |ctx| {
                query::bind_query_pipeline(ctx, first)
            })?;
            for (_, pipeline) in rest {
                ctx.with_child_scope(ScopeKind::Projection, pipeline.span, true, |ctx| {
                    query::bind_query_pipeline(ctx, pipeline)
                })?;
            }
        }
        Statement::Chained { blocks, .. } => {
            for block in blocks {
                ctx.with_child_scope(ScopeKind::Projection, block.span, true, |ctx| {
                    query::bind_query_pipeline(ctx, block)
                })?;
            }
        }
        Statement::Mutate(pipeline) => mutation::bind_mutation_pipeline(&mut ctx, pipeline)?,
        Statement::Ddl(statement) => ddl::bind_ddl_statement(&mut ctx, statement)?,
        Statement::Call(call) => {
            call::bind_procedure_call(&mut ctx, call)?;
        }
        Statement::StartTransaction { span }
        | Statement::Commit { span }
        | Statement::Rollback { span } => transaction::bind_transaction_control(&mut ctx, *span),
    }
    Ok(ctx.finish(stmt))
}

pub(crate) struct BindContext {
    scopes: BindingScopeTree,
    current: ScopeId,
    references: Vec<BindingUse>,
    yield_stars: Vec<SourceSpan>,
}

impl BindContext {
    fn new(root_span: SourceSpan) -> Self {
        let scopes = BindingScopeTree::new(root_span);
        let current = scopes.root();
        Self {
            scopes,
            current,
            references: Vec::new(),
            yield_stars: Vec::new(),
        }
    }

    fn finish(self, stmt: Statement) -> AnalyzedStatement {
        AnalyzedStatement::new(stmt, self.scopes, self.references, self.yield_stars)
    }

    pub(crate) fn declare_strict(
        &mut self,
        kind: BindingDeclKind,
        name: IStr,
        span: SourceSpan,
    ) -> Result<BindingId, AnalysisError> {
        self.scopes.declare_strict(self.current, kind, name, span)
    }

    pub(crate) fn declare_or_reuse(
        &mut self,
        kind: BindingDeclKind,
        name: IStr,
        span: SourceSpan,
    ) -> BindingId {
        let (binding, reused) = self.scopes.declare_or_reuse(self.current, kind, name, span);
        if reused {
            self.references.push(BindingUse {
                name,
                binding,
                span,
                kind: BindingUseKind::PatternReuse,
            });
        }
        binding
    }

    pub(crate) fn resolve(
        &mut self,
        name: IStr,
        span: SourceSpan,
        kind: BindingUseKind,
    ) -> Result<BindingId, AnalysisError> {
        let Some(binding) = self.scopes.resolve(self.current, name) else {
            return Err(AnalysisError::undefined_reference(name, span));
        };
        self.references.push(BindingUse {
            name,
            binding,
            span,
            kind,
        });
        Ok(binding)
    }

    pub(crate) fn with_child_scope<T>(
        &mut self,
        kind: ScopeKind,
        span: SourceSpan,
        boundary: bool,
        f: impl FnOnce(&mut Self) -> Result<T, AnalysisError>,
    ) -> Result<T, AnalysisError> {
        let parent = self.current;
        let child = self.scopes.push_scope(parent, kind, span, boundary);
        self.current = child;
        let result = f(self);
        self.current = parent;
        result
    }

    pub(crate) fn record_yield_star(&mut self, span: SourceSpan) {
        self.yield_stars.push(span);
    }

    pub(crate) fn enter_projection_scope(&mut self, span: SourceSpan) {
        let child = self
            .scopes
            .push_scope(self.current, ScopeKind::Projection, span, true);
        self.current = child;
    }
}
