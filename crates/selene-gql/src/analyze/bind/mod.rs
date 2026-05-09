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
    ProcedureRegistry, SourceSpan, Statement, ValueExpr,
    analyze::{
        ast::AnalyzedStatement,
        binding::{BindingDeclKind, BindingId, BindingUse, BindingUseKind},
        category::{self, StatementCategory},
        error::AnalysisError,
        scope::{BindingScopeTree, ScopeId, ScopeKind},
        types::{AnalyzedType, ExprId, ExprIdMap, ExprTypeTable},
        write_set::{self, MutationWriteSet},
    },
};

/// Analyze one statement with the BRIEF-21 binding pass.
pub(crate) fn bind_statement(
    stmt: Statement,
    registry: &dyn ProcedureRegistry,
) -> Result<AnalyzedStatement, AnalysisError> {
    let mut ctx = BindContext::new(stmt.span(), registry);
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
    let category = category::classify(&stmt, registry);
    let write_set = match &stmt {
        Statement::Mutate(pipeline) => Some(write_set::compute_write_set(
            pipeline,
            &ctx.scopes,
            &ctx.references,
        )),
        _ => None,
    };
    Ok(ctx.finish(stmt, category, write_set))
}

pub(crate) struct BindContext<'ctx> {
    scopes: BindingScopeTree,
    current: ScopeId,
    references: Vec<BindingUse>,
    expr_types: ExprTypeTable,
    expr_ids: ExprIdMap,
    registry: &'ctx dyn ProcedureRegistry,
}

impl<'ctx> BindContext<'ctx> {
    fn new(root_span: SourceSpan, registry: &'ctx dyn ProcedureRegistry) -> Self {
        let scopes = BindingScopeTree::new(root_span);
        let current = scopes.root();
        Self {
            scopes,
            current,
            references: Vec::new(),
            expr_types: ExprTypeTable::default(),
            expr_ids: ExprIdMap::default(),
            registry,
        }
    }

    fn finish(
        self,
        stmt: Statement,
        category: StatementCategory,
        write_set: Option<MutationWriteSet>,
    ) -> AnalyzedStatement {
        AnalyzedStatement::new(
            stmt,
            self.scopes,
            self.references,
            self.expr_types,
            self.expr_ids,
            category,
            write_set,
        )
    }

    pub(crate) fn declare_strict_typed(
        &mut self,
        kind: BindingDeclKind,
        name: IStr,
        span: SourceSpan,
        ty: AnalyzedType,
    ) -> Result<BindingId, AnalysisError> {
        self.scopes
            .declare_strict_typed(self.current, kind, name, span, ty)
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

    pub(crate) fn allocate_expr(&mut self, expr: &ValueExpr, ty: AnalyzedType) -> ExprId {
        let id = self.expr_types.push(ty);
        self.expr_ids.insert(expr, id);
        id
    }

    pub(crate) fn expr_type(&self, id: ExprId) -> &AnalyzedType {
        self.expr_types.get(id)
    }

    pub(crate) fn expr_id(&self, expr: &ValueExpr) -> Option<ExprId> {
        self.expr_ids.get(expr)
    }

    pub(crate) fn binding_type(&self, binding: BindingId) -> AnalyzedType {
        self.scopes
            .declaration(binding)
            .map(|decl| decl.ty().clone())
            .unwrap_or(AnalyzedType::Dynamic)
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

    pub(crate) fn registry(&self) -> &'ctx dyn ProcedureRegistry {
        self.registry
    }
}
