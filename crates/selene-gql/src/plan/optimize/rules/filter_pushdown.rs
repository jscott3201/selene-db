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
    let mut changed = false;
    while let Some(PipelineOp::Filter(pred)) = plan.pipeline.first() {
        if !pred
            .binding_refs
            .iter()
            .all(|binding| pattern_bindings.contains(binding))
        {
            break;
        }
        let PipelineOp::Filter(pred) = plan.pipeline.remove(0) else {
            unreachable!("first op was already checked as Filter");
        };
        pattern.filters.push(pred);
        changed = true;
    }
    changed
}

fn binding_set(bindings: &[BindingDef]) -> BTreeSet<BindingId> {
    bindings.iter().map(|binding| binding.binding).collect()
}
