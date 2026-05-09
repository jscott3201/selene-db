//! Boolean conjunction splitting rule.

use crate::{
    BinaryOp, GqlType, ValueExpr,
    analyze::AnalyzedType,
    plan::{
        BindingDef, ExecutionPlan, FilterPredicate, FilterPredicateKind, PipelineOp,
        optimize::{OptimizeContext, Rule, Transformed, binding_refs, walk},
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
        let pattern_bindings = plan
            .pattern_plan
            .as_ref()
            .map(|pattern| pattern.bindings.clone())
            .unwrap_or_default();
        if plan.pattern_plan.is_some() {
            let mut filters = {
                let pattern = plan.pattern_plan.as_mut().expect("pattern checked");
                std::mem::take(&mut pattern.filters)
            };
            changed |= split_predicate_vec(&mut filters, &pattern_bindings, &mut plan);
            plan.pattern_plan.as_mut().expect("pattern checked").filters = filters;
        }
        let mut pipeline = std::mem::take(&mut plan.pipeline);
        changed |= split_pipeline_filters(&mut pipeline, &pattern_bindings, &mut plan);
        plan.pipeline = pipeline;

        let nested = walk::recurse_rule_subplans(plan, self, ctx);
        changed |= nested.changed;
        Transformed {
            plan: nested.plan,
            changed,
        }
    }
}

fn split_pipeline_filters(
    pipeline: &mut Vec<PipelineOp>,
    bindings: &[BindingDef],
    plan: &mut ExecutionPlan,
) -> bool {
    let mut changed = false;
    let mut rewritten = Vec::with_capacity(pipeline.len());
    for op in pipeline.drain(..) {
        match op {
            PipelineOp::Filter(pred) => {
                let predicates = split_predicate(pred, bindings, plan);
                changed |= predicates.len() > 1;
                rewritten.extend(predicates.into_iter().map(PipelineOp::Filter));
            }
            other => rewritten.push(other),
        }
    }
    *pipeline = rewritten;
    changed
}

fn split_predicate_vec(
    predicates: &mut Vec<FilterPredicate>,
    bindings: &[BindingDef],
    plan: &mut ExecutionPlan,
) -> bool {
    let mut changed = false;
    let mut rewritten = Vec::with_capacity(predicates.len());
    for pred in predicates.drain(..) {
        let split = split_predicate(pred, bindings, plan);
        changed |= split.len() > 1;
        rewritten.extend(split);
    }
    *predicates = rewritten;
    changed
}

fn split_predicate(
    pred: FilterPredicate,
    bindings: &[BindingDef],
    plan: &mut ExecutionPlan,
) -> Vec<FilterPredicate> {
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
        .map(|expr| {
            let binding_refs = binding_refs::collect_binding_refs(&expr, bindings)
                .unwrap_or_else(|| pred.binding_refs.clone());
            FilterPredicate {
                span: expr.span(),
                expr,
                expr_id: plan.alloc_expr_id(),
                ty: AnalyzedType::Resolved(GqlType::Boolean),
                binding_refs,
                kind: FilterPredicateKind::Expression,
                index_consumed: false,
            }
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
