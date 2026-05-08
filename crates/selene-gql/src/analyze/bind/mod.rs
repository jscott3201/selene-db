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
            // Each NEXT block consumes the prior block's binding table, so
            // we run sequential blocks in scopes chained off the previous
            // block's *terminal* scope (which holds the projection aliases
            // each block published). Boundary stays `false` so post-RETURN
            // bindings flow forward through GA07.
            let root = ctx.current_scope();
            let mut prior_tail = root;
            for block in blocks {
                let block_root =
                    ctx.scopes
                        .push_scope(prior_tail, ScopeKind::Projection, block.span, false);
                ctx.set_scope(block_root);
                query::bind_query_pipeline(&mut ctx, block)?;
                prior_tail = ctx.current_scope();
            }
            ctx.set_scope(root);
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
    ) -> Result<BindingId, AnalysisError> {
        let (binding, reused) = self
            .scopes
            .declare_or_reuse(self.current, kind, name, span)?;
        if reused {
            self.references.push(BindingUse {
                name,
                binding,
                span,
                kind: BindingUseKind::PatternReuse,
            });
        }
        Ok(binding)
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

    /// Enter a fresh projection scope and stay there for the rest of the
    /// pipeline.
    ///
    /// `boundary = false` matches ISO Feature `GA07` ("Ordering by discarded
    /// binding variables") for `RETURN`-style projections: pre-projection
    /// bindings stay reachable for downstream `ORDER BY` / `OFFSET` /
    /// `LIMIT`, and `RETURN *` keeps the entire input row visible.
    ///
    /// `boundary = true` matches the `WITH` continuation rule: pre-`WITH`
    /// bindings end at the boundary and only the WITH-projected aliases
    /// flow into the next clauses.
    pub(crate) fn enter_projection_scope(&mut self, span: SourceSpan, boundary: bool) {
        let child = self
            .scopes
            .push_scope(self.current, ScopeKind::Projection, span, boundary);
        self.current = child;
    }

    pub(crate) fn current_scope(&self) -> ScopeId {
        self.current
    }

    pub(crate) fn set_scope(&mut self, scope: ScopeId) {
        self.current = scope;
    }
}
