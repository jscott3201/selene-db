//! Optimizer helpers for binding and property-expression recognition.

use selene_core::IStr;

use crate::{
    BinaryOp, GqlType, IsCheckKind, Literal, SourceSpan, ValueExpr,
    analyze::BindingId,
    plan::{BindingDef, FilterPredicate, FilterPredicateKind},
};

/// Parameter reference borrowed out of a [`ValueExpr::Parameter`] node.
///
/// Mirrors the shape index-aware optimizer rules need when admitting parameter
/// slots into typed-index / composite-index / bitmap-union access paths
/// (BRIEF-154 §B.2). The `declared_type` borrow lets call sites run plan-time
/// typed-incompatibility checks without cloning [`GqlType`].
#[derive(Clone, Copy, Debug)]
// Commit 1 lands the helper surface; Commit 2 wires the first caller
// (`compatible_value` in `range_index_scan`). Until then the struct is
// dead-code from the compiler's perspective.
#[allow(dead_code)]
pub(crate) struct ParameterRef<'a> {
    /// Parameter name without the leading `$`.
    pub name: IStr,
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
    pub key: IStr,
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
        // names (e.g., `EXISTS ((n)-[:r]->(b))` references outer `n`) without
        // emitting an explicit `Variable` expression. Trigger the unresolved
        // fallback so callers preserve the parent's annotated `binding_refs`
        // instead of silently dropping the subquery's outer references.
        ValueExpr::Exists { .. } | ValueExpr::CountSubquery { .. } => {
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
            key: *key,
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
) -> Option<(BindingId, IStr)> {
    let ValueExpr::PropertyAccess { target, key, .. } = expr else {
        return None;
    };
    let ValueExpr::Variable { name, .. } = target.as_ref() else {
        return None;
    };
    bindings
        .iter()
        .find(|binding| binding.name == *name)
        .map(|binding| (binding.binding, *key))
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
// Commit 1 lands the helper; Commit 2 wires the first caller. Allow the
// `dead_code` warning until then so `-D warnings` clippy stays green.
#[allow(dead_code)]
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
        name: *name,
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
    visit(expr);
    match expr {
        ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => {}
        ValueExpr::PropertyAccess { target, .. } => walk_expr(target, visit),
        ValueExpr::ListAccess { target, index, .. } => {
            walk_expr(target, visit);
            walk_expr(index, visit);
        }
        ValueExpr::ListLiteral { items, .. } => {
            for item in items {
                walk_expr(item, visit);
            }
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                walk_expr(value, visit);
            }
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            walk_expr(lhs, visit);
            walk_expr(rhs, visit);
        }
        ValueExpr::UnaryOp { operand, .. } => walk_expr(operand, visit),
        ValueExpr::FunctionCall { args, .. } => {
            for arg in args {
                walk_expr(arg, visit);
            }
        }
        ValueExpr::Normalize { source, .. } => walk_expr(source, visit),
        ValueExpr::Trim {
            character, source, ..
        } => {
            if let Some(character) = character {
                walk_expr(character, visit);
            }
            walk_expr(source, visit);
        }
        ValueExpr::IsCheck { operand, kind, .. } => {
            walk_expr(operand, visit);
            // `n IS SOURCE OF e` / `n IS DESTINATION OF e` bind the edge in
            // `kind`; that operand must be walked so the collected binding-refs
            // include the edge (e.g. `{n, e}`). Omitting it lets the optimizer's
            // filter pushdown treat the predicate as single-binding and push it
            // onto the node scan/expand before the edge is bound. Mirrors
            // `plan::optimize::walk::walk_is_check`.
            match kind {
                IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
                    walk_expr(value, visit);
                }
                IsCheckKind::Null
                | IsCheckKind::Directed
                | IsCheckKind::Labeled(_)
                | IsCheckKind::TruthValue(_)
                | IsCheckKind::Typed(_)
                | IsCheckKind::Normalized(_) => {}
            }
        }
        ValueExpr::InList { operand, list, .. } => {
            walk_expr(operand, visit);
            for item in list {
                walk_expr(item, visit);
            }
        }
        ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => {
            for item in items {
                walk_expr(item, visit);
            }
        }
        ValueExpr::PropertyExists { target, .. } => walk_expr(target, visit),
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (when, then) in branches {
                walk_expr(when, visit);
                walk_expr(then, visit);
            }
            if let Some(value) = else_branch {
                walk_expr(value, visit);
            }
        }
        ValueExpr::Cast { value, .. } => walk_expr(value, visit),
        ValueExpr::Exists { .. }
        | ValueExpr::CountSubquery { .. }
        | ValueExpr::ValueSubquery { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::types::AnalyzedType;
    use crate::plan::BindingElement;

    // `intern_with_admission` (not the budget-bypassing `intern`) keeps this
    // test module clear of the `no_unbudgeted_intern_call_in_selene_gql` guard.
    fn name_of(name: &str) -> selene_core::IStr {
        selene_core::intern_with_admission(name).unwrap().0
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
