//! Planned expression-subquery IR.

use std::collections::BTreeMap;

use crate::{
    SourceSpan,
    analyze::{BindingId, ExprId},
};

use super::PatternPlan;

/// Planned subquery referenced by an expression ID in a containing plan.
///
/// `Exists` follows ISO/IEC 39075:2024 section 19.4. `CountSubquery` is a
/// selene-db dialect extension over a single `MATCH` pattern; it counts every
/// row produced by that pattern, including duplicates from join shapes.
#[derive(Clone, Debug)]
pub struct PlannedSubquery {
    /// Subquery expression kind.
    pub kind: SubqueryKind,
    /// Lowered pattern executed for the subquery body.
    pub plan: PatternPlan,
    /// Outer-scope bindings referenced by the inner pattern, sorted and deduped.
    pub outer_binding_refs: Vec<BindingId>,
    /// Source span of the subquery expression.
    pub span: SourceSpan,
}

/// Planned expression-subquery kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubqueryKind {
    /// ISO GQL `EXISTS` predicate.
    Exists {
        /// Whether the source was `NOT EXISTS`.
        negated: bool,
    },
    /// selene-db `COUNT { MATCH ... }` dialect extension.
    Count,
}

/// Plan-level registry of expression subqueries indexed by AST expression ID.
///
/// The analyzer allocates expression IDs, lowering clones that lookup table
/// onto the execution plan, and runtime evaluation uses this registry as the
/// single source of truth for subquery bodies.
#[derive(Clone, Debug, Default)]
pub struct SubqueryRegistry {
    by_expr_id: BTreeMap<ExprId, PlannedSubquery>,
}

impl SubqueryRegistry {
    /// Insert or replace the planned subquery for `expr_id`.
    pub fn insert(&mut self, expr_id: ExprId, subquery: PlannedSubquery) {
        self.by_expr_id.insert(expr_id, subquery);
    }

    /// Return the planned subquery for `expr_id`, if any.
    #[must_use]
    pub fn get(&self, expr_id: ExprId) -> Option<&PlannedSubquery> {
        self.by_expr_id.get(&expr_id)
    }

    /// Return true when no subqueries were planned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_expr_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        PatternPlan, SourceSpan,
        analyze::ExprId,
        plan::{JoinTree, PlannedSubquery, SubqueryKind, SubqueryRegistry},
    };

    fn planned_subquery() -> PlannedSubquery {
        PlannedSubquery {
            kind: SubqueryKind::Count,
            plan: PatternPlan {
                bindings: Vec::new(),
                join_tree: JoinTree::WorstCaseOptimal {
                    intersection: Vec::new(),
                    node_id_ordering: Vec::new(),
                },
                filters: Vec::new(),
                paths: Vec::new(),
            },
            outer_binding_refs: Vec::new(),
            span: SourceSpan::default(),
        }
    }

    #[test]
    fn registry_insert_get_and_is_empty() {
        let mut registry = SubqueryRegistry::default();
        let expr_id = ExprId::new(7);

        assert!(registry.is_empty());
        registry.insert(expr_id, planned_subquery());

        assert!(!registry.is_empty());
        assert!(matches!(
            registry.get(expr_id).map(|subquery| subquery.kind),
            Some(SubqueryKind::Count)
        ));
        assert!(registry.get(ExprId::new(8)).is_none());
    }
}
