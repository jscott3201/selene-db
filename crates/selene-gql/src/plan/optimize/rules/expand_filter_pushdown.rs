//! Pattern-level filter pushdown into edge expansions.

use crate::{
    analyze::BindingId,
    plan::{
        ExecutionPlan, FilterPredicate,
        optimize::{OptimizeContext, Rule, Transformed, walk},
    },
};

/// Move single-edge-binding predicates onto their matching `Expand` edge.
pub struct ExpandFilterPushdown;

impl Rule for ExpandFilterPushdown {
    fn name(&self) -> &'static str {
        "expand_filter_pushdown"
    }

    fn rewrite(
        &self,
        mut plan: ExecutionPlan,
        ctx: &OptimizeContext<'_>,
    ) -> Transformed<ExecutionPlan> {
        let mut changed = false;
        if let Some(pattern) = &mut plan.pattern_plan {
            changed |= push_pattern_filters(pattern);
        }
        let nested = walk::recurse_rule_subplans(plan, self, ctx);
        changed |= nested.changed;
        Transformed {
            plan: nested.plan,
            changed,
        }
    }
}

fn push_pattern_filters(pattern: &mut crate::PatternPlan) -> bool {
    let mut changed = false;
    let mut remaining = Vec::with_capacity(pattern.filters.len());
    for pred in pattern.filters.drain(..) {
        let Some(binding) = single_binding(&pred) else {
            remaining.push(pred);
            continue;
        };
        let mut pending = Some(pred);
        let pushed = walk::walk_expand_nodes(&mut pattern.join_tree, &mut |edge| {
            if pending.is_some() && edge.binding == Some(binding) {
                edge.property_predicates
                    .push(pending.take().expect("pending predicate exists"));
                true
            } else {
                false
            }
        });
        if pushed {
            changed = true;
        } else {
            remaining.push(pending.expect("predicate was not pushed"));
        }
    }
    pattern.filters = remaining;
    changed
}

fn single_binding(pred: &FilterPredicate) -> Option<BindingId> {
    match pred.binding_refs.as_slice() {
        [binding] => Some(*binding),
        _ => None,
    }
}
