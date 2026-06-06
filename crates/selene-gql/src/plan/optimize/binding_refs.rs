//! Optimizer helpers for binding and property-expression recognition.

use selene_core::DbString;

use crate::{
    BinaryOp, GqlType, Literal, SourceSpan, ValueExpr,
    analyze::BindingId,
    plan::{BindingDef, FilterPredicate, FilterPredicateKind},
};

/// Parameter reference borrowed out of a [`ValueExpr::Parameter`] node.
///
/// Mirrors the shape index-aware optimizer rules need when admitting parameter
/// slots into typed-index / composite-index / bitmap-union access paths
/// (BRIEF-154 §B.2). The `declared_type` borrow lets call sites run plan-time
/// typed-incompatibility checks without cloning [`GqlType`].
#[derive(Clone, Debug)]
pub(crate) struct ParameterRef<'a> {
    /// Parameter name without the leading `$`.
    pub name: DbString,
    /// Optional declared type from a `$id :: TYPE` annotation (BRIEF-137).
    pub declared_type: Option<&'a GqlType>,
    /// Source span of the parameter reference.
    pub span: SourceSpan,
}

/// Property predicate shape recognized by index-aware optimizer rules.
#[derive(Clone, Debug)]
pub(crate) enum PropertyPredicateShape<'a> {
    /// `binding.key = literal`.
    Equality(&'a ValueExpr),
    /// `binding.key <op> literal`, normalized so `op` compares property to literal.
    Comparison {
        /// Normalized comparison operator.
        op: BinaryOp,
        /// Literal-side expression.
        value: &'a ValueExpr,
    },
    /// `binding.key IN [items]`.
    InList(Vec<&'a ValueExpr>),
}

/// Matched property predicate.
#[derive(Clone, Debug)]
pub(crate) struct MatchedPropertyPredicate<'a> {
    /// Referenced binding.
    pub binding: BindingId,
    /// Property key.
    pub key: DbString,
    /// Predicate shape.
    pub shape: PropertyPredicateShape<'a>,
}

/// Recompute referenced binding IDs by matching variable names against binding definitions.
pub(crate) fn collect_binding_refs(
    expr: &ValueExpr,
    bindings: &[BindingDef],
) -> Option<Vec<BindingId>> {
    let mut refs = Vec::new();
    let mut unresolved = false;
    walk_expr(expr, &mut |sub| match sub {
        ValueExpr::Variable { name, .. } => {
            if let Some(binding) = bindings
                .iter()
                .find(|binding| binding.name == *name)
                .map(|binding| binding.binding)
            {
                refs.push(binding);
            } else {
                unresolved = true;
            }
        }
        // Subquery patterns can reference outer bindings via pattern element
        // names (e.g., `EXISTS ((n)-[:r]->(b))` references outer `n`) or via a
        // body expression, without emitting an explicit `Variable` expression
        // at this level (the walker treats subquery bodies as opaque). Trigger
        // the unresolved fallback so callers preserve the parent's annotated
        // `binding_refs` instead of silently dropping the subquery's outer
        // references — a sibling rewrite must not strip a ValueSubquery's
        // correlated refs.
        ValueExpr::Exists { .. }
        | ValueExpr::CountSubquery { .. }
        | ValueExpr::ValueSubquery { .. } => {
            unresolved = true;
        }
        _ => {}
    });
    if unresolved {
        return None;
    }
    refs.sort();
    refs.dedup();
    Some(refs)
}

/// Recognize a property predicate against a known binding table.
pub(crate) fn match_property_predicate<'a>(
    pred: &'a FilterPredicate,
    bindings: &'a [BindingDef],
) -> Option<MatchedPropertyPredicate<'a>> {
    match &pred.kind {
        FilterPredicateKind::PropertyEquals {
            binding: Some(binding),
            key,
        } => Some(MatchedPropertyPredicate {
            binding: *binding,
            key: key.clone(),
            shape: PropertyPredicateShape::Equality(&pred.expr),
        }),
        FilterPredicateKind::Expression => match_property_expr(&pred.expr, bindings),
        FilterPredicateKind::PropertyEquals { binding: None, .. } => None,
    }
}

/// Return the binding/key for a plain property access expression.
pub(crate) fn match_property_access(
    expr: &ValueExpr,
    bindings: &[BindingDef],
) -> Option<(BindingId, DbString)> {
    let ValueExpr::PropertyAccess { target, key, .. } = expr else {
        return None;
    };
    let ValueExpr::Variable { name, .. } = target.as_ref() else {
        return None;
    };
    bindings
        .iter()
        .find(|binding| binding.name == *name)
        .map(|binding| (binding.binding, key.clone()))
}

/// Return `expr` as a non-null literal.
pub(crate) fn literal(expr: &ValueExpr) -> Option<&Literal> {
    let ValueExpr::Literal(literal) = expr else {
        return None;
    };
    (!matches!(literal, Literal::Null(_))).then_some(literal)
}

/// Return `expr` as a parameter reference, borrowing the inline
/// `declared_type` so callers may run plan-time typed-incompatibility checks
/// without cloning.
///
/// `binding_refs::parameter` is intentionally permissive about NULL parameter
/// declarations: untyped slots (the BRIEF-115 baseline) and typed slots
/// (BRIEF-137 `$id :: TYPE`) are both returned. Plan-time and execute-time
/// validation lives in the optimizer rules and runtime resolver respectively.
pub(crate) fn parameter(expr: &ValueExpr) -> Option<ParameterRef<'_>> {
    let ValueExpr::Parameter {
        name,
        declared_type,
        span,
    } = expr
    else {
        return None;
    };
    Some(ParameterRef {
        name: name.clone(),
        declared_type: declared_type.as_ref(),
        span: *span,
    })
}

fn match_property_expr<'a>(
    expr: &'a ValueExpr,
    bindings: &'a [BindingDef],
) -> Option<MatchedPropertyPredicate<'a>> {
    match expr {
        ValueExpr::BinaryOp { op, lhs, rhs, .. }
            if matches!(
                op,
                BinaryOp::Eq | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
            ) =>
        {
            match_binary_property(*op, lhs, rhs, bindings)
        }
        ValueExpr::InList {
            operand,
            list,
            negated: false,
            ..
        } => {
            let (binding, key) = match_property_access(operand, bindings)?;
            Some(MatchedPropertyPredicate {
                binding,
                key,
                shape: PropertyPredicateShape::InList(list.iter().collect()),
            })
        }
        _ => None,
    }
}

fn match_binary_property<'a>(
    op: BinaryOp,
    lhs: &'a ValueExpr,
    rhs: &'a ValueExpr,
    bindings: &'a [BindingDef],
) -> Option<MatchedPropertyPredicate<'a>> {
    if let Some((binding, key)) = match_property_access(lhs, bindings) {
        return Some(MatchedPropertyPredicate {
            binding,
            key,
            shape: match op {
                BinaryOp::Eq => PropertyPredicateShape::Equality(rhs),
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                    PropertyPredicateShape::Comparison { op, value: rhs }
                }
                _ => return None,
            },
        });
    }

    let (binding, key) = match_property_access(rhs, bindings)?;
    Some(MatchedPropertyPredicate {
        binding,
        key,
        shape: match op {
            BinaryOp::Eq => PropertyPredicateShape::Equality(lhs),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                PropertyPredicateShape::Comparison {
                    op: reverse_comparison(op),
                    value: lhs,
                }
            }
            _ => return None,
        },
    })
}

fn reverse_comparison(op: BinaryOp) -> BinaryOp {
    match op {
        BinaryOp::Lt => BinaryOp::Gt,
        BinaryOp::Le => BinaryOp::Ge,
        BinaryOp::Gt => BinaryOp::Lt,
        BinaryOp::Ge => BinaryOp::Le,
        other => other,
    }
}

fn walk_expr(expr: &ValueExpr, visit: &mut impl FnMut(&ValueExpr)) {
    // Pre-order: visit this node, then recurse into direct `ValueExpr` children.
    //
    // `for_each_child` yields the `IS SOURCE OF` / `IS DESTINATION OF` operand as
    // a child. That operand must be walked so collected binding-refs include the
    // edge it binds (e.g. `{n, e}`); omitting it would let the optimizer's filter
    // pushdown treat the predicate as single-binding and push it onto the node
    // scan/expand before the edge is bound. Subquery variants
    // (`Exists`/`CountSubquery`/`ValueSubquery`) carry no `ValueExpr` children and
    // are deliberately not descended here — their outer-binding uses are
    // collected separately by the caller.
    visit(expr);
    expr.for_each_child(&mut |child| walk_expr(child, visit));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IsCheckKind;
    use crate::analyze::types::AnalyzedType;
    use crate::plan::BindingElement;

    fn name_of(name: &str) -> selene_core::DbString {
        selene_core::db_string(name).unwrap()
    }

    fn binding_def(name: &str, raw: u32, element: BindingElement) -> BindingDef {
        BindingDef {
            binding: BindingId::new(raw),
            name: name_of(name),
            element,
            ty: AnalyzedType::Dynamic,
            label_predicate: None,
            span: SourceSpan::new(0, 1),
        }
    }

    fn var(name: &str) -> ValueExpr {
        ValueExpr::Variable {
            name: name_of(name),
            span: SourceSpan::new(0, 1),
        }
    }

    // PLAN-01: `n IS SOURCE OF e` / `n IS DESTINATION OF e` reference BOTH the
    // node operand and the edge bound inside `kind`. The collector must yield
    // both bindings; if it dropped the edge, `expand_filter_pushdown` would push
    // the predicate onto `n`'s scan before `e` is bound (a plan-correctness bug).
    #[test]
    fn collect_binding_refs_includes_edge_in_is_source_of() {
        let bindings = [
            binding_def("n", 0, BindingElement::Node),
            binding_def("e", 1, BindingElement::Edge),
        ];
        let expr = ValueExpr::IsCheck {
            operand: Box::new(var("n")),
            kind: IsCheckKind::SourceOf(Box::new(var("e"))),
            negated: false,
            span: SourceSpan::new(0, 1),
        };
        let refs = collect_binding_refs(&expr, &bindings).expect("all variables resolve");
        assert_eq!(refs, vec![BindingId::new(0), BindingId::new(1)]);
    }

    #[test]
    fn collect_binding_refs_includes_edge_in_is_destination_of() {
        let bindings = [
            binding_def("n", 0, BindingElement::Node),
            binding_def("e", 1, BindingElement::Edge),
        ];
        let expr = ValueExpr::IsCheck {
            operand: Box::new(var("n")),
            kind: IsCheckKind::DestinationOf(Box::new(var("e"))),
            negated: false,
            span: SourceSpan::new(0, 1),
        };
        let refs = collect_binding_refs(&expr, &bindings).expect("all variables resolve");
        assert_eq!(refs, vec![BindingId::new(0), BindingId::new(1)]);
    }
}
