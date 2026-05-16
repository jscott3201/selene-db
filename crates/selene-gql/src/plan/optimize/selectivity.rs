//! Predicate selectivity heuristics.

use selene_core::IStr;

use crate::{
    BinaryOp, LabelExpr, ValueExpr,
    plan::{
        BindingDef, EdgeStatistics, FilterPredicate, FilterPredicateKind, optimize::OptimizeContext,
    },
};

use super::binding_refs::match_property_predicate;

const HEURISTIC_EQUALS: f64 = 0.05;
const HEURISTIC_NEQ: f64 = 0.95;
const HEURISTIC_RANGE: f64 = 0.33;
const HEURISTIC_OR: f64 = 0.66;
const HEURISTIC_EXISTS: f64 = 0.5;
const HEURISTIC_DEFAULT: f64 = 0.5;

/// Predicate-estimation context for one pattern plan.
pub(crate) struct ScanContext<'a> {
    /// Pattern binding definitions.
    pub bindings: &'a [BindingDef],
    /// Optional graph statistics.
    pub statistics: Option<&'a EdgeStatistics>,
}

/// Estimate predicate selectivity. Lower values are more selective.
pub(crate) fn estimate(
    pred: &FilterPredicate,
    _ctx: &OptimizeContext<'_>,
    scan_ctx: &ScanContext<'_>,
) -> f64 {
    if let Some(stats) = scan_ctx.statistics
        && let Some(matched) = match_property_predicate(pred, scan_ctx.bindings)
        && let Some(label) = label_for_binding(scan_ctx.bindings, matched.binding)
        && let Some(histogram) = stats.property_histograms.get(&(label, matched.key))
    {
        return histogram.estimate_for(&pred.expr);
    }

    match pred.kind {
        FilterPredicateKind::PropertyEquals { .. } => HEURISTIC_EQUALS,
        FilterPredicateKind::Expression => estimate_expr(&pred.expr),
    }
}

fn estimate_expr(expr: &ValueExpr) -> f64 {
    match expr {
        ValueExpr::BinaryOp {
            op: BinaryOp::Eq, ..
        } => HEURISTIC_EQUALS,
        ValueExpr::BinaryOp {
            op: BinaryOp::Ne, ..
        } => HEURISTIC_NEQ,
        ValueExpr::BinaryOp {
            op: BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge,
            ..
        } => HEURISTIC_RANGE,
        ValueExpr::BinaryOp {
            op: BinaryOp::Or, ..
        } => HEURISTIC_OR,
        ValueExpr::BinaryOp {
            op: BinaryOp::And,
            lhs,
            rhs,
            ..
        } => estimate_expr(lhs) * estimate_expr(rhs),
        ValueExpr::InList { list, .. } => {
            let k = list.len().max(1) as i32;
            1.0 - (1.0 - HEURISTIC_EQUALS).powi(k)
        }
        ValueExpr::Exists { .. } | ValueExpr::PropertyExists { .. } => HEURISTIC_EXISTS,
        _ => HEURISTIC_DEFAULT,
    }
}

fn label_for_binding(bindings: &[BindingDef], binding_id: crate::BindingId) -> Option<IStr> {
    bindings
        .iter()
        .find(|binding| binding.binding == binding_id)
        .and_then(|binding| match &binding.label_predicate {
            Some(LabelExpr::Single(label)) => Some(*label),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ExprId, FilterPredicateKind, Literal, SourceSpan, analyze::AnalyzedType,
        plan::FilterPredicate,
    };

    fn in_list_predicate(len: usize) -> FilterPredicate {
        let span = SourceSpan::new(0, 1);
        FilterPredicate {
            expr: ValueExpr::InList {
                operand: Box::new(ValueExpr::Literal(Literal::Integer(1, span))),
                list: (0..len)
                    .map(|index| ValueExpr::Literal(Literal::Integer(index as i64, span)))
                    .collect(),
                negated: false,
                span,
            },
            expr_id: ExprId::new(0),
            ty: AnalyzedType::DYNAMIC,
            binding_refs: Vec::new(),
            kind: FilterPredicateKind::Expression,
            index_consumed: false,
            span,
        }
    }

    #[test]
    fn selectivity_in_list_caps_at_one() {
        let ctx = OptimizeContext::default();
        let scan_ctx = ScanContext {
            bindings: &[],
            statistics: None,
        };

        let estimate = estimate(&in_list_predicate(100), &ctx, &scan_ctx);

        assert!(estimate <= 1.0);
    }

    #[test]
    fn selectivity_in_list_ranks_between_eq_and_neq_for_small_k() {
        let ctx = OptimizeContext::default();
        let scan_ctx = ScanContext {
            bindings: &[],
            statistics: None,
        };

        let estimate = estimate(&in_list_predicate(10), &ctx, &scan_ctx);

        assert!((estimate - 0.401_263_060_761_621).abs() < 1e-12);
        assert!(estimate > HEURISTIC_EQUALS);
        assert!(estimate < HEURISTIC_NEQ);
    }
}
