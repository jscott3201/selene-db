//! Planner optimizer entry points.
//!
//! The optimizer is intentionally infallible: lowering owns planner errors,
//! while rules either rewrite a plan or leave it unchanged.

mod binding_refs;
mod context;
mod cost;
mod index_catalog;
mod live_index_catalog;
mod registry;
mod rule;
pub mod rules;
mod selectivity;
#[cfg(any(test, feature = "test-harness"))]
mod summary;
mod walk;

pub use context::OptimizeContext;
pub use index_catalog::{
    CompositeIndexHandle, EmptyIndexCatalog, IndexCatalog, IndexHandle, IndexKind, IndexTarget,
    TypedIndexLookup,
};
pub use live_index_catalog::LiveIndexCatalog;
pub use registry::{DEFAULT_RULES, RULE_NAMES};
pub use rule::{Rule, Transformed};
#[cfg(any(test, feature = "test-harness"))]
pub use summary::{
    PatternSnapshot, PipelineOpSummary, PlanSnapshot, ScanSnapshot, optimize_summary,
};
// Test-harness-gated re-export of the cost estimators so integration tests can
// exercise them in isolation against a synthetic catalog. Production code
// reaches these via the `cost` module path; this is not a stable public API
// (gated exactly like `summary`). Kept out of `src/` test modules because the
// `dos_guard` interner-budget scan rejects any direct interner-call substring
// under `src/`.
#[cfg(any(test, feature = "test-harness"))]
pub use cost::{
    composite_cost, disjunctive_cost, in_list_cost, linear_baseline, should_decline_index,
    typed_index_cost,
};

use crate::plan::ExecutionPlan;

/// Optimize an execution plan using the default structural rule set.
#[must_use]
#[tracing::instrument(
    name = "selene.gql.optimize",
    skip(plan, ctx),
    fields(category = ?plan.category)
)]
pub fn optimize(plan: ExecutionPlan, ctx: &OptimizeContext<'_>) -> ExecutionPlan {
    optimize_with_rules(plan, DEFAULT_RULES, ctx)
}

/// Optimize an execution plan using an explicit rule set.
///
/// This seam exists for defensive fixed-point tests that need custom rules
/// without exposing rule registration as public API.
pub(crate) fn optimize_with_rules(
    mut plan: ExecutionPlan,
    rules: &[&'static dyn Rule],
    ctx: &OptimizeContext<'_>,
) -> ExecutionPlan {
    for _ in 0..ctx.impl_defined_caps.max_optimizer_iterations {
        let mut changed = false;
        for rule in rules {
            let transformed = rule.rewrite(plan, ctx);
            plan = transformed.plan;
            changed |= transformed.changed;
        }
        if !changed {
            plan.refresh_pipeline_op_high_water();
            return plan;
        }
    }
    plan.refresh_pipeline_op_high_water();
    plan
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::{
        EmptyProcedureRegistry, ImplDefinedCaps, PipelineOp, StatementCategory, analyze, parse,
        plan::{
            BindingTableSchema, ExecutionPlan,
            optimize::{OptimizeContext, Rule, Transformed, optimize, optimize_with_rules},
            plan,
        },
    };

    fn empty_plan(caps: ImplDefinedCaps) -> ExecutionPlan {
        ExecutionPlan {
            category: StatementCategory::ReadOnly,
            pattern_plan: None,
            pipeline: Vec::new(),
            output_schema: BindingTableSchema {
                columns: Vec::new(),
            },
            impl_defined_caps: caps,
            expr_ids: Default::default(),
            subqueries: Default::default(),
            next_expr_id: crate::ExprId::new(0),
            next_pipeline_op_id: crate::PipelineOpId::new(0),
        }
    }

    fn plan_one(source: &str) -> ExecutionPlan {
        let statement = parse(source).expect("test input parses");
        let analyzed =
            analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
        plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
    }

    #[test]
    fn no_rules_return_original_plan() {
        let caps = ImplDefinedCaps::default();
        let ctx = OptimizeContext::new(&caps);
        let plan = empty_plan(caps);
        let before = format!("{plan:?}");
        let optimized = optimize_with_rules(plan, &[], &ctx);
        assert_eq!(format!("{optimized:?}"), before);
    }

    #[test]
    fn default_optimizer_is_idempotent_after_convergence() {
        let plan = plan_one("MATCH (n) FILTER n.age > 30 RETURN n ORDER BY n.age LIMIT 10");
        let once = optimize(plan, &OptimizeContext::default());
        let twice = optimize(once.clone(), &OptimizeContext::default());
        assert_eq!(format!("{once:?}"), format!("{twice:?}"));
    }

    #[test]
    fn iteration_cap_stops_oscillating_rules() {
        struct AlwaysChanged {
            calls: AtomicUsize,
        }

        impl Rule for AlwaysChanged {
            fn name(&self) -> &'static str {
                "always_changed"
            }

            fn rewrite(
                &self,
                mut plan: ExecutionPlan,
                _ctx: &OptimizeContext<'_>,
            ) -> Transformed<ExecutionPlan> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if plan.pipeline.is_empty() {
                    plan.pipeline.push(PipelineOp::Distinct);
                } else {
                    plan.pipeline.clear();
                }
                Transformed::changed(plan)
            }
        }

        let caps = ImplDefinedCaps {
            max_optimizer_iterations: 3,
            ..ImplDefinedCaps::default()
        };
        let ctx = OptimizeContext::new(&caps);
        let rule = Box::leak(Box::new(AlwaysChanged {
            calls: AtomicUsize::new(0),
        }));
        let rules: [&'static dyn Rule; 1] = [rule];

        let optimized = optimize_with_rules(empty_plan(caps), &rules, &ctx);

        assert_eq!(rule.calls.load(Ordering::SeqCst), 3);
        assert!(
            optimized.pipeline.is_empty() || matches!(optimized.pipeline[0], PipelineOp::Distinct)
        );
    }

    #[test]
    fn optimizer_refreshes_pipeline_op_id_high_water_after_rewrite() {
        struct AppendDistinct {
            calls: AtomicUsize,
        }

        impl Rule for AppendDistinct {
            fn name(&self) -> &'static str {
                "append_distinct"
            }

            fn rewrite(
                &self,
                mut plan: ExecutionPlan,
                _ctx: &OptimizeContext<'_>,
            ) -> Transformed<ExecutionPlan> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    plan.pipeline.push(PipelineOp::Distinct);
                    Transformed::changed(plan)
                } else {
                    Transformed::unchanged(plan)
                }
            }
        }

        let caps = ImplDefinedCaps::default();
        let ctx = OptimizeContext::new(&caps);
        let rule = Box::leak(Box::new(AppendDistinct {
            calls: AtomicUsize::new(0),
        }));
        let rules: [&'static dyn Rule; 1] = [rule];

        let optimized = optimize_with_rules(empty_plan(caps), &rules, &ctx);

        assert_eq!(optimized.pipeline.len(), 1);
        assert_eq!(
            optimized.next_pipeline_op_id.get(),
            optimized.pipeline.len() as u32
        );
    }
}
