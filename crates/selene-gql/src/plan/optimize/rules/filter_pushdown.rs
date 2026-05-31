//! Pipeline-to-pattern filter pushdown.

use std::collections::BTreeSet;

use crate::{
    analyze::BindingId,
    plan::{
        BindingDef, ExecutionPlan, PipelineOp,
        optimize::{OptimizeContext, Rule, Transformed, walk},
    },
};

/// Move leading pipeline filters into the leading pattern phase when safe.
pub struct FilterPushdown;

impl Rule for FilterPushdown {
    fn name(&self) -> &'static str {
        "filter_pushdown"
    }

    fn rewrite(
        &self,
        mut plan: ExecutionPlan,
        ctx: &OptimizeContext<'_>,
    ) -> Transformed<ExecutionPlan> {
        let mut changed = push_leading_filters(&mut plan);
        let nested = walk::recurse_rule_subplans(plan, self, ctx);
        changed |= nested.changed;
        Transformed {
            plan: nested.plan,
            changed,
        }
    }
}

fn push_leading_filters(plan: &mut ExecutionPlan) -> bool {
    let Some(pattern) = &mut plan.pattern_plan else {
        return false;
    };
    let pattern_bindings = binding_set(&pattern.bindings);
    // Count the maximal leading run of pushable `Filter` ops (each a `Filter`
    // whose binding-refs are all satisfied by the pattern), then move the whole
    // prefix in one `drain` — avoiding the per-iteration `remove(0)` O(n²) shift.
    let count = plan
        .pipeline
        .iter()
        .take_while(|op| match op {
            PipelineOp::Filter(pred) => pred
                .binding_refs
                .iter()
                .all(|binding| pattern_bindings.contains(binding)),
            _ => false,
        })
        .count();
    if count == 0 {
        return false;
    }
    for op in plan.pipeline.drain(0..count) {
        let PipelineOp::Filter(pred) = op else {
            unreachable!("prefix ops were already checked as pushable Filters");
        };
        pattern.filters.push(pred);
    }
    true
}

fn binding_set(bindings: &[BindingDef]) -> BTreeSet<BindingId> {
    bindings.iter().map(|binding| binding.binding).collect()
}
