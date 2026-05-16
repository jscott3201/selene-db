//! Selectivity-based predicate reordering.

use std::cmp::Ordering;

use crate::plan::{
    BindingDef, ExecutionPlan, FilterPredicate, JoinTree,
    optimize::{OptimizeContext, Rule, Transformed, selectivity, walk},
};

/// Reorder predicate buckets by estimated selectivity.
pub struct PredicateReorder;

impl Rule for PredicateReorder {
    fn name(&self) -> &'static str {
        "predicate_reorder"
    }

    fn rewrite(
        &self,
        mut plan: ExecutionPlan,
        ctx: &OptimizeContext<'_>,
    ) -> Transformed<ExecutionPlan> {
        let mut changed = false;
        if let Some(pattern) = &mut plan.pattern_plan {
            let scan_ctx = selectivity::ScanContext {
                bindings: &pattern.bindings,
                statistics: ctx.statistics,
            };
            changed |= reorder_bucket(&mut pattern.filters, ctx, &scan_ctx);
            changed |= reorder_tree(&mut pattern.join_tree, &pattern.bindings, ctx);
        }
        let nested = walk::recurse_rule_subplans(plan, self, ctx);
        changed |= nested.changed;
        Transformed {
            plan: nested.plan,
            changed,
        }
    }
}

fn reorder_tree(tree: &mut JoinTree, bindings: &[BindingDef], ctx: &OptimizeContext<'_>) -> bool {
    let scan_ctx = selectivity::ScanContext {
        bindings,
        statistics: ctx.statistics,
    };
    match tree {
        JoinTree::Scan(scan) => reorder_bucket(&mut scan.property_predicates, ctx, &scan_ctx),
        JoinTree::Expand { child, edge, .. } => {
            reorder_tree(child, bindings, ctx)
                | reorder_bucket(&mut edge.property_predicates, ctx, &scan_ctx)
                | reorder_bucket(&mut edge.right_property_predicates, ctx, &scan_ctx)
        }
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            reorder_tree(left, bindings, ctx) | reorder_tree(right, bindings, ctx)
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => false,
    }
}

fn reorder_bucket(
    predicates: &mut [FilterPredicate],
    ctx: &OptimizeContext<'_>,
    scan_ctx: &selectivity::ScanContext<'_>,
) -> bool {
    let before = predicates
        .iter()
        .map(|pred| pred.expr_id)
        .collect::<Vec<_>>();
    predicates.sort_by(|left, right| {
        selectivity::estimate(left, ctx, scan_ctx)
            .partial_cmp(&selectivity::estimate(right, ctx, scan_ctx))
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.expr_id.get().cmp(&right.expr_id.get()))
    });
    let after = predicates
        .iter()
        .map(|pred| pred.expr_id)
        .collect::<Vec<_>>();
    after != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExprId, FilterPredicateKind, Literal, SourceSpan, analyze::AnalyzedType};

    fn ne_predicate(expr_id: u32) -> FilterPredicate {
        let span = SourceSpan::new(0, 1);
        FilterPredicate {
            expr: crate::ValueExpr::BinaryOp {
                op: crate::BinaryOp::Ne,
                lhs: Box::new(crate::ValueExpr::Literal(Literal::Integer(1, span))),
                rhs: Box::new(crate::ValueExpr::Literal(Literal::Integer(2, span))),
                span,
            },
            expr_id: ExprId::new(expr_id),
            ty: AnalyzedType::DYNAMIC,
            binding_refs: Vec::new(),
            kind: FilterPredicateKind::Expression,
            index_consumed: false,
            span,
        }
    }

    #[test]
    fn predicate_reorder_canonical_under_equal_selectivity() {
        let mut predicates = vec![ne_predicate(3), ne_predicate(1), ne_predicate(2)];
        let ctx = OptimizeContext::default();
        let scan_ctx = selectivity::ScanContext {
            bindings: &[],
            statistics: None,
        };

        let changed = reorder_bucket(&mut predicates, &ctx, &scan_ctx);

        // ExprId is stable within a plan but can be re-issued by rules that
        // synthesize expressions; the tie-break only canonicalizes one run.
        assert!(changed);
        assert_eq!(
            predicates
                .iter()
                .map(|predicate| predicate.expr_id.get())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }
}
