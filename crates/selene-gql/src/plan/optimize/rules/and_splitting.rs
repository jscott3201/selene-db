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
        // The pattern bindings are only read while splitting an actual `AND`
        // filter (to recollect per-conjunct binding-refs). When neither the
        // pattern filters nor the pipeline filters contain a top-level `AND`,
        // splitting is a no-op, so skip cloning the bindings entirely.
        let pattern_bindings = if plan_has_and_filter(&plan) {
            plan.pattern_plan
                .as_ref()
                .map(|pattern| pattern.bindings.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
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

/// True when the plan has at least one splittable (`AND`-expression) filter,
/// either in the leading pattern plan or in the top-level pipeline. Used to
/// skip the pattern-binding clone when splitting would be a no-op.
fn plan_has_and_filter(plan: &ExecutionPlan) -> bool {
    let pattern_has = plan
        .pattern_plan
        .as_ref()
        .is_some_and(|pattern| pattern.filters.iter().any(is_and_expression_filter));
    pattern_has
        || plan.pipeline.iter().any(|op| match op {
            PipelineOp::Filter(pred) => is_and_expression_filter(pred),
            _ => false,
        })
}

fn is_and_expression_filter(pred: &FilterPredicate) -> bool {
    pred.kind == FilterPredicateKind::Expression
        && matches!(
            pred.expr,
            ValueExpr::BinaryOp {
                op: BinaryOp::And,
                ..
            }
        )
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
    // Guard before the deep clone: only a top-level `AND` flattens to >1
    // predicate (a non-`AND` pushes exactly one, so `flatten_and` would return
    // `len <= 1`). Checking the discriminant first avoids cloning the whole
    // expression tree for the common single-predicate case.
    if !matches!(
        pred.expr,
        ValueExpr::BinaryOp {
            op: BinaryOp::And,
            ..
        }
    ) {
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
