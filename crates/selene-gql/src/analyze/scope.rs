//! Binding scope tree.

use selene_core::IStr;

use crate::{
    SourceSpan,
    analyze::{
        binding::{BindingDecl, BindingDeclKind, BindingId},
        error::{AnalysisError, PatternElementKind},
        types::AnalyzedType,
    },
};

/// Stable index of a lexical binding scope.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ScopeId(u32);

impl ScopeId {
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return this scope's zero-based numeric index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Metadata kind for a binding scope.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScopeKind {
    /// Root statement scope.
    Statement,
    /// Isolated projection boundary created by `RETURN` or `WITH`.
    Projection,
    /// Nested subquery scope created by `EXISTS` or `COUNT { ... }`.
    Subquery,
    /// Diagnostic scope for a `CASE` branch.
    CaseBranch,
}

/// One lexical scope in a [`BindingScopeTree`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingScope {
    /// Parent scope index; root has no parent.
    pub parent: Option<ScopeId>,
    /// Declaration indexes owned by this scope.
    pub locals: Vec<BindingId>,
    /// Best-effort source extent for this scope.
    pub span: SourceSpan,
    /// Diagnostic scope kind.
    pub kind: ScopeKind,
    /// Whether lookup should stop before walking to the parent.
    pub boundary: bool,
}

/// Lexical binding scopes allocated during one analyzer call.
#[derive(Clone, Debug)]
pub struct BindingScopeTree {
    decls: Vec<BindingDecl>,
    scopes: Vec<BindingScope>,
}

impl BindingScopeTree {
    /// Create a tree with a single root statement scope.
    #[must_use]
    pub fn new(root_span: SourceSpan) -> Self {
        Self {
            decls: Vec::new(),
            scopes: vec![BindingScope {
                parent: None,
                locals: Vec::new(),
                span: root_span,
                kind: ScopeKind::Statement,
                boundary: false,
            }],
        }
    }

    /// Return the root scope ID.
    #[must_use]
    pub const fn root(&self) -> ScopeId {
        ScopeId::new(0)
    }

    /// Return all declarations in allocation order.
    #[must_use]
    pub fn declarations(&self) -> &[BindingDecl] {
        &self.decls
    }

    /// Return all scopes in allocation order.
    #[must_use]
    pub fn scopes(&self) -> &[BindingScope] {
        &self.scopes
    }

    /// Return the declaration for a binding ID.
    #[must_use]
    pub fn declaration(&self, id: BindingId) -> Option<&BindingDecl> {
        self.decls.get(id.get() as usize)
    }

    /// Return a scope by ID.
    #[must_use]
    pub fn scope(&self, id: ScopeId) -> Option<&BindingScope> {
        self.scopes.get(id.get() as usize)
    }

    pub(crate) fn push_scope(
        &mut self,
        parent: ScopeId,
        kind: ScopeKind,
        span: SourceSpan,
        boundary: bool,
    ) -> ScopeId {
        let id = ScopeId::new(self.scopes.len() as u32);
        self.scopes.push(BindingScope {
            parent: Some(parent),
            locals: Vec::new(),
            span,
            kind,
            boundary,
        });
        id
    }

    pub(crate) fn declare_strict_typed(
        &mut self,
        scope: ScopeId,
        kind: BindingDeclKind,
        name: IStr,
        span: SourceSpan,
        ty: AnalyzedType,
    ) -> Result<BindingId, AnalysisError> {
        if let Some(prior) = self.resolve_local(scope, name) {
            let prior_span = self
                .declaration(prior)
                .map(BindingDecl::span)
                .unwrap_or_default();
            return Err(AnalysisError::Shadow {
                name,
                span,
                prior_span,
            });
        }
        Ok(self.declare_unchecked(scope, kind, name, span, ty, None))
    }

    pub(crate) fn declare_or_reuse_with_labels(
        &mut self,
        scope: ScopeId,
        kind: BindingDeclKind,
        name: IStr,
        span: SourceSpan,
        labels: Option<crate::LabelExpr>,
    ) -> Result<(BindingId, bool), AnalysisError> {
        if let Some(existing) = self.resolve(scope, name) {
            // Cross-element-kind reuse is a semantic error: a node variable
            // cannot be silently rebound as an edge/path variable, and vice
            // versa. Same-element-kind reuse (NodePattern <-> InsertNode,
            // EdgePattern <-> InsertEdge) is the legitimate path that lets
            // `MATCH (n) INSERT (n)-[:K]->(m)` work.
            let prior_decl = self
                .declaration(existing)
                .expect("resolved binding has decl");
            let new_element = PatternElementKind::from_decl_kind(kind);
            let prior_element = PatternElementKind::from_decl_kind(prior_decl.kind());
            if let (Some(new_kind), Some(prior_kind)) = (new_element, prior_element)
                && new_kind != prior_kind
            {
                return Err(AnalysisError::PatternKindMismatch {
                    name,
                    prior: prior_kind,
                    current: new_kind,
                    span,
                    prior_span: prior_decl.span(),
                });
            }
            return Ok((existing, true));
        }
        Ok((
            self.declare_unchecked(
                scope,
                kind,
                name,
                span,
                BindingDecl::default_type(kind),
                labels,
            ),
            false,
        ))
    }

    pub(crate) fn resolve(&self, scope: ScopeId, name: IStr) -> Option<BindingId> {
        let mut cursor = Some(scope);
        while let Some(scope_id) = cursor {
            let scope = self.scope(scope_id)?;
            if let Some(id) = self.resolve_local(scope_id, name) {
                return Some(id);
            }
            if scope.boundary {
                return None;
            }
            cursor = scope.parent;
        }
        None
    }

    fn declare_unchecked(
        &mut self,
        scope: ScopeId,
        kind: BindingDeclKind,
        name: IStr,
        span: SourceSpan,
        ty: AnalyzedType,
        labels: Option<crate::LabelExpr>,
    ) -> BindingId {
        let id = BindingId::new(self.decls.len() as u32);
        self.decls
            .push(BindingDecl::new(kind, id, name, span, ty, labels));
        self.scopes[scope.get() as usize].locals.push(id);
        id
    }

    fn resolve_local(&self, scope: ScopeId, name: IStr) -> Option<BindingId> {
        self.scope(scope)?.locals.iter().copied().find(|id| {
            self.declaration(*id)
                .is_some_and(|decl| decl.name() == name)
        })
    }
}
