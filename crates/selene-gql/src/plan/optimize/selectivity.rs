//! Predicate selectivity heuristics.
//!
//! `estimate` returns a selectivity *fraction* in `[0, 1]` (lower = more
//! selective); `predicate_reorder` sorts residual filters ascending by it so the
//! cheapest predicate runs first. When the optimizer is given a live
//! [`IndexCatalog`] (OPT-5), property-equality predicates against a single-label
//! binding are estimated from live statistics (`equality_cardinality /
//! label_cardinality`); every other shape, and the no-catalog case, falls back
//! to the verbatim `HEURISTIC_*` constants + `expr_id` tie-break so plans are
//! byte-identical to pre-OPT-5 HEAD when stats are absent.

use selene_core::{DbString, Value};

use crate::{
    BinaryOp, LabelExpr, Literal, ValueExpr,
    plan::{
        BindingDef, FilterPredicate, FilterPredicateKind, IndexTarget, optimize::IndexCatalog,
        optimize::OptimizeContext,
    },
};

use super::binding_refs::{self, match_property_predicate};

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
    /// Optional live index catalog supplying cost statistics (OPT-5).
    pub catalog: Option<&'a dyn IndexCatalog>,
}

/// Estimate predicate selectivity. Lower values are more selective.
pub(crate) fn estimate(
    pred: &FilterPredicate,
    _ctx: &OptimizeContext<'_>,
    scan_ctx: &ScanContext<'_>,
) -> f64 {
    if let Some(selectivity) = stats_selectivity(pred, scan_ctx) {
        return selectivity;
    }

    match pred.kind {
        FilterPredicateKind::PropertyEquals { .. } => HEURISTIC_EQUALS,
        FilterPredicateKind::Expression => estimate_expr(&pred.expr),
    }
}

/// Live-statistics selectivity for a property-equality predicate against a
/// single-label node binding: `equality_cardinality / label_cardinality`.
///
/// Returns `None` (caller falls back to the heuristic) whenever any piece is
/// unavailable: no catalog, non-equality shape, parameter value (unknown at plan
/// time), missing label, or an absent/empty label population.
fn stats_selectivity(pred: &FilterPredicate, scan_ctx: &ScanContext<'_>) -> Option<f64> {
    let catalog = scan_ctx.catalog?;
    let matched = match_property_predicate(pred, scan_ctx.bindings)?;
    let label = label_for_binding(scan_ctx.bindings, matched.binding)?;
    let binding_refs::PropertyPredicateShape::Equality(value_expr) = matched.shape else {
        return None;
    };
    // Only literal equality has an exact count at plan time; parameters defer
    // to the heuristic so the no-value case stays unchanged.
    let value = equality_literal_value(value_expr)?;
    let matches =
        catalog.equality_cardinality(IndexTarget::Node, label.clone(), matched.key, &value)?;
    let population = catalog.label_cardinality(IndexTarget::Node, label)?;
    if population == 0 {
        return None;
    }
    Some((matches as f64 / population as f64).clamp(0.0, 1.0))
}

/// Lower a literal-equality value expression to a concrete [`Value`], or `None`
/// for parameters / nulls.
fn equality_literal_value(expr: &ValueExpr) -> Option<Value> {
    let literal = binding_refs::literal(expr)?;
    match literal {
        Literal::Bool(value, _) => Some(Value::Bool(*value)),
        Literal::Integer(value, _) => Some(Value::Int(*value)),
        Literal::Float(value, _) => Some(Value::Float(*value)),
        Literal::String(value, _) => Some(Value::String(value.clone())),
        Literal::Bytes(value, _) => Some(Value::Bytes(value.clone())),
        Literal::Uuid(value, _) => Some(Value::Uuid(*value)),
        Literal::ZonedDateTime(value, _) => Some(Value::ZonedDateTime(value.clone())),
        Literal::LocalDateTime(value, _) => Some(Value::LocalDateTime(*value)),
        Literal::Date(value, _) => Some(Value::Date(*value)),
        Literal::ZonedTime(value, _) => Some(Value::ZonedTime(value.clone())),
        Literal::LocalTime(value, _) => Some(Value::LocalTime(*value)),
        Literal::Duration(value, _) => Some(Value::Duration(value.clone())),
        Literal::Null(_) => None,
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

fn label_for_binding(bindings: &[BindingDef], binding_id: crate::BindingId) -> Option<DbString> {
    bindings
        .iter()
        .find(|binding| binding.binding == binding_id)
        .and_then(|binding| match &binding.label_predicate {
            Some(LabelExpr::Single(label)) => Some(label.clone()),
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
            catalog: None,
        };

        let estimate = estimate(&in_list_predicate(100), &ctx, &scan_ctx);

        assert!(estimate <= 1.0);
    }

    #[test]
    fn selectivity_in_list_ranks_between_eq_and_neq_for_small_k() {
        let ctx = OptimizeContext::default();
        let scan_ctx = ScanContext {
            bindings: &[],
            catalog: None,
        };

        let estimate = estimate(&in_list_predicate(10), &ctx, &scan_ctx);

        assert!((estimate - 0.401_263_060_761_621).abs() < 1e-12);
        assert!(estimate > HEURISTIC_EQUALS);
        assert!(estimate < HEURISTIC_NEQ);
    }
}
