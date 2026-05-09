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
    {
        return stats
            .property_histograms
            .get(&(label, matched.key))
            .map(|histogram| histogram.estimate_for(&pred.expr))
            .unwrap_or(HEURISTIC_EQUALS);
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
        ValueExpr::InList { list, .. } => HEURISTIC_EQUALS * (list.len() as f64).max(1.0),
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
