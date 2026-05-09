//! Adjacent `OrderBy` + `Limit` fusion.

use crate::plan::{
    ExecutionPlan, LimitAmount, PipelineOp,
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
    let mut iter = std::mem::take(pipeline).into_iter().peekable();

    while let Some(op) = iter.next() {
        match op {
            PipelineOp::OrderBy(keys) if next_limit_is_bounded(iter.peek()) => {
                let Some(PipelineOp::Limit { offset, count }) = iter.next() else {
                    unreachable!("peek confirmed the next op is Limit");
                };
                rewritten.push(PipelineOp::TopK {
                    keys,
                    offset,
                    count,
                });
                changed = true;
            }
            other => rewritten.push(other),
        }
    }

    *pipeline = rewritten;
    changed
}

fn next_limit_is_bounded(op: Option<&PipelineOp>) -> bool {
    matches!(
        op,
        Some(PipelineOp::Limit { count, .. })
            if !matches!(count, LimitAmount::Literal(u64::MAX))
    )
}
