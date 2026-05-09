//! Boolean conjunction splitting rule.

use crate::{
    BinaryOp, GqlType, ValueExpr,
    analyze::AnalyzedType,
    plan::{
        ExecutionPlan, FilterPredicate, FilterPredicateKind, PipelineOp,
        optimize::{OptimizeContext, Rule, Transformed, walk},
    },
};

/// Split boolean `AND` filters into adjacent predicates.
pub struct AndSplitting;

impl Rule for AndSplitting {
    fn name(&self) -> &'static str {
        "and_splitting"
    }

    fn rewrite(
        &self,
        mut plan: ExecutionPlan,
        ctx: &OptimizeContext<'_>,
    ) -> Transformed<ExecutionPlan> {
        let mut changed = false;
        if let Some(pattern) = &mut plan.pattern_plan {
            changed |= split_predicate_vec(&mut pattern.filters);
        }
        changed |= split_pipeline_filters(&mut plan.pipeline);

        let nested = walk::recurse_rule_subplans(plan, self, ctx);
        changed |= nested.changed;
        Transformed {
            plan: nested.plan,
            changed,
        }
    }
}

fn split_pipeline_filters(pipeline: &mut Vec<PipelineOp>) -> bool {
    let mut changed = false;
    let mut rewritten = Vec::with_capacity(pipeline.len());
    for op in pipeline.drain(..) {
        match op {
            PipelineOp::Filter(pred) => {
                let predicates = split_predicate(pred);
                changed |= predicates.len() > 1;
                rewritten.extend(predicates.into_iter().map(PipelineOp::Filter));
            }
            other => rewritten.push(other),
        }
    }
    *pipeline = rewritten;
    changed
}

fn split_predicate_vec(predicates: &mut Vec<FilterPredicate>) -> bool {
    let mut changed = false;
    let mut rewritten = Vec::with_capacity(predicates.len());
    for pred in predicates.drain(..) {
        let split = split_predicate(pred);
        changed |= split.len() > 1;
        rewritten.extend(split);
    }
    *predicates = rewritten;
    changed
}

fn split_predicate(pred: FilterPredicate) -> Vec<FilterPredicate> {
    if pred.kind != FilterPredicateKind::Expression {
        return vec![pred];
    }
    let mut exprs = Vec::new();
    flatten_and(pred.expr.clone(), &mut exprs);
    if exprs.len() <= 1 {
        return vec![pred];
    }

    exprs
        .into_iter()
        .map(|expr| FilterPredicate {
            span: expr.span(),
            expr,
            expr_id: pred.expr_id,
            ty: AnalyzedType::Resolved(GqlType::Boolean),
            binding_refs: pred.binding_refs.clone(),
            kind: FilterPredicateKind::Expression,
        })
        .collect()
}

fn flatten_and(expr: ValueExpr, out: &mut Vec<ValueExpr>) {
    match expr {
        ValueExpr::BinaryOp {
            op: BinaryOp::And,
            lhs,
            rhs,
            ..
        } => {
            flatten_and(*lhs, out);
            flatten_and(*rhs, out);
        }
        other => out.push(other),
    }
}
