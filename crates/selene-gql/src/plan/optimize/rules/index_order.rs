//! Sort-key typed-index access hints.

use crate::plan::{
    BindingDef, ExecutionPlan, OrderAccess, OrderKey, PipelineOp,
    optimize::{OptimizeContext, Rule, Transformed, binding_refs, walk},
};

use super::index_helpers::binding_index_target;

/// Annotate order keys that can read from a typed index.
pub struct IndexOrder;

impl Rule for IndexOrder {
    fn name(&self) -> &'static str {
        "index_order"
    }

    fn rewrite(
        &self,
        mut plan: ExecutionPlan,
        ctx: &OptimizeContext<'_>,
    ) -> Transformed<ExecutionPlan> {
        let Some(catalog) = ctx.index_catalog else {
            return Transformed::unchanged(plan);
        };
        let mut changed = false;
        if let Some(pattern) = &plan.pattern_plan {
            for op in &mut plan.pipeline {
                changed |= rewrite_pipeline_op(op, &pattern.bindings, catalog);
            }
        }
        let nested = walk::recurse_rule_subplans(plan, self, ctx);
        changed |= nested.changed;
        Transformed {
            plan: nested.plan,
            changed,
        }
    }
}

fn rewrite_pipeline_op(
    op: &mut PipelineOp,
    bindings: &[BindingDef],
    catalog: &dyn crate::IndexCatalog,
) -> bool {
    match op {
        PipelineOp::OrderBy(keys) | PipelineOp::TopK { keys, .. } => {
            rewrite_keys(keys, bindings, catalog)
        }
        PipelineOp::Filter(_)
        | PipelineOp::Project(_)
        | PipelineOp::Let(_)
        | PipelineOp::Unwind { .. }
        | PipelineOp::Limit { .. }
        | PipelineOp::GroupBy { .. }
        | PipelineOp::Distinct
        | PipelineOp::Match(_)
        | PipelineOp::Union { .. }
        | PipelineOp::Chain(_)
        | PipelineOp::Call(_)
        | PipelineOp::CallSubquery(_)
        | PipelineOp::Mutation(_)
        | PipelineOp::Catalog(_)
        | PipelineOp::ExplainPlan { .. }
        | PipelineOp::Tx(_) => false,
    }
}

fn rewrite_keys(
    keys: &mut [OrderKey],
    bindings: &[BindingDef],
    catalog: &dyn crate::IndexCatalog,
) -> bool {
    let mut changed = false;
    for key in keys {
        let Some((binding, property)) = binding_refs::match_property_access(&key.expr, bindings)
        else {
            break;
        };
        let Some((target, label)) = binding_index_target(bindings, binding) else {
            break;
        };
        let Some(lookup) = catalog.typed_index(target, label, property) else {
            break;
        };
        let access = OrderAccess::TypedIndex {
            handle: lookup.handle,
            direction: key.direction,
        };
        if key.access.as_ref() != Some(&access) {
            key.access = Some(access);
            changed = true;
        }
    }
    changed
}
