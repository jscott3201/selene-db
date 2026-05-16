//! Adjacent `OrderBy` + `Limit` fusion.

use crate::plan::{
    ExecutionPlan, LimitAmount, OrderKey, PipelineOp, ProjectExpr,
    optimize::{OptimizeContext, Rule, Transformed, walk},
};

/// Fuse adjacent sort and bounded-limit operations into `TopK`.
pub struct TopK;

impl Rule for TopK {
    fn name(&self) -> &'static str {
        "top_k"
    }

    fn rewrite(
        &self,
        mut plan: ExecutionPlan,
        ctx: &OptimizeContext<'_>,
    ) -> Transformed<ExecutionPlan> {
        let mut changed = fuse_pipeline(&mut plan.pipeline);
        let nested = walk::recurse_rule_subplans(plan, self, ctx);
        changed |= nested.changed;
        Transformed {
            plan: nested.plan,
            changed,
        }
    }
}

fn fuse_pipeline(pipeline: &mut Vec<PipelineOp>) -> bool {
    let mut changed = false;
    let mut rewritten = Vec::with_capacity(pipeline.len());
    let input = std::mem::take(pipeline);
    let mut index = 0;

    while index < input.len() {
        if let (
            Some(PipelineOp::Project(projects)),
            Some(PipelineOp::OrderBy(keys)),
            Some(PipelineOp::Limit { offset, count }),
        ) = (input.get(index), input.get(index + 1), input.get(index + 2))
            && limit_is_bounded(count)
            && keys_can_run_before_project(projects, keys)
        {
            rewritten.push(PipelineOp::TopK {
                keys: keys.clone(),
                offset: offset.clone(),
                count: count.clone(),
            });
            rewritten.push(PipelineOp::Project(projects.clone()));
            changed = true;
            index += 3;
            continue;
        }

        if let (Some(PipelineOp::OrderBy(keys)), Some(PipelineOp::Limit { offset, count })) =
            (input.get(index), input.get(index + 1))
            && limit_is_bounded(count)
        {
            rewritten.push(PipelineOp::TopK {
                keys: keys.clone(),
                offset: offset.clone(),
                count: count.clone(),
            });
            changed = true;
            index += 2;
            continue;
        }

        rewritten.push(input[index].clone());
        index += 1;
    }

    *pipeline = rewritten;
    changed
}

fn limit_is_bounded(count: &LimitAmount) -> bool {
    !matches!(count, LimitAmount::Literal(u64::MAX))
}

fn keys_can_run_before_project(projects: &[ProjectExpr], keys: &[OrderKey]) -> bool {
    let mut projected_input_refs = projects
        .iter()
        .flat_map(|project| project.binding_refs.iter().copied())
        .collect::<Vec<_>>();
    projected_input_refs.sort_unstable();
    projected_input_refs.dedup();

    keys.iter()
        .flat_map(|key| key.binding_refs.iter().copied())
        .all(|binding| projected_input_refs.binary_search(&binding).is_ok())
}
