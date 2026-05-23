//! Typed range/equality index scan rule.

use crate::{
    BinaryOp, Literal,
    plan::{
        BindingDef, ExecutionPlan, FilterPredicate, JoinTree, ScanAccess, ScanKind,
        TypedIndexBounds,
        optimize::{OptimizeContext, Rule, Transformed, binding_refs, walk},
    },
};

use super::index_helpers::{literal_matches_kind, single_label};

/// Rewrite property equality/range predicates to typed index access.
pub struct RangeIndexScan;

impl Rule for RangeIndexScan {
    fn name(&self) -> &'static str {
        "range_index_scan"
    }

    fn rewrite(
        &self,
        mut plan: ExecutionPlan,
        ctx: &OptimizeContext<'_>,
    ) -> Transformed<ExecutionPlan> {
        let Some(catalog) = ctx.index_catalog else {
            return Transformed::unchanged(plan);
        };
        let mut changed = false;
        if let Some(pattern) = &mut plan.pattern_plan {
            changed |= rewrite_tree(&mut pattern.join_tree, &pattern.bindings, catalog);
        }
        let nested = walk::recurse_rule_subplans(plan, self, ctx);
        changed |= nested.changed;
        Transformed {
            plan: nested.plan,
            changed,
        }
    }
}

fn rewrite_tree(
    tree: &mut JoinTree,
    bindings: &[BindingDef],
    catalog: &dyn crate::IndexCatalog,
) -> bool {
    match tree {
        JoinTree::Scan(scan) => rewrite_scan(scan, bindings, catalog),
        JoinTree::Expand { child, .. } | JoinTree::Repeat { child, .. } => {
            rewrite_tree(child, bindings, catalog)
        }
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            rewrite_tree(left, bindings, catalog) | rewrite_tree(right, bindings, catalog)
        }
        JoinTree::PathSearch { child, .. } => rewrite_tree(child, bindings, catalog),
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => false,
    }
}

fn rewrite_scan(
    scan: &mut crate::NodeOrEdgeScan,
    bindings: &[BindingDef],
    catalog: &dyn crate::IndexCatalog,
) -> bool {
    if scan.kind != ScanKind::Node || !matches!(scan.access, ScanAccess::Linear) {
        return false;
    }
    let Some(label) = single_label(&scan.label_predicate) else {
        return false;
    };
    let Some(candidate) = best_candidate(&scan.property_predicates, bindings, catalog, label)
    else {
        return false;
    };
    remove_indices(&mut scan.property_predicates, &candidate.consumed_indices);
    scan.access = ScanAccess::TypedIndexRange {
        handle: candidate.handle,
        property: candidate.property,
        kind: candidate.kind,
        bounds: candidate.bounds,
    };
    true
}

struct Candidate {
    handle: crate::IndexHandle,
    property: selene_core::IStr,
    kind: crate::IndexKind,
    bounds: TypedIndexBounds,
    consumed_indices: Vec<usize>,
}

fn best_candidate(
    predicates: &[FilterPredicate],
    bindings: &[BindingDef],
    catalog: &dyn crate::IndexCatalog,
    label: selene_core::IStr,
) -> Option<Candidate> {
    for (index, pred) in predicates.iter().enumerate() {
        let Some(matched) = binding_refs::match_property_predicate(pred, bindings) else {
            continue;
        };
        if !binding_is_node(bindings, matched.binding) {
            continue;
        }
        let Some(lookup) = catalog.typed_index(crate::IndexTarget::Node, label, matched.key) else {
            continue;
        };
        let Some((bounds, mut consumed_indices)) =
            bounds_for_property(matched.key, predicates, bindings, index, lookup.kind)
        else {
            continue;
        };
        consumed_indices.sort_unstable();
        consumed_indices.dedup();
        return Some(Candidate {
            handle: lookup.handle,
            property: matched.key,
            kind: lookup.kind,
            bounds,
            consumed_indices,
        });
    }
    None
}

fn bounds_for_property(
    key: selene_core::IStr,
    predicates: &[FilterPredicate],
    bindings: &[BindingDef],
    first_index: usize,
    kind: crate::IndexKind,
) -> Option<(TypedIndexBounds, Vec<usize>)> {
    let mut equality = None;
    let mut lower: Option<(Literal, bool)> = None;
    let mut upper: Option<(Literal, bool)> = None;
    let mut consumed = Vec::new();

    for (index, pred) in predicates.iter().enumerate().skip(first_index) {
        let Some(matched) = binding_refs::match_property_predicate(pred, bindings) else {
            continue;
        };
        if matched.key != key {
            continue;
        }
        match matched.shape {
            binding_refs::PropertyPredicateShape::Equality(value) => {
                let literal = compatible_literal(value, kind)?;
                equality = Some(literal.clone());
                consumed.push(index);
                break;
            }
            binding_refs::PropertyPredicateShape::Comparison { op, value } => {
                let literal = compatible_literal(value, kind)?;
                let candidate = (literal.clone(), matches!(op, BinaryOp::Ge | BinaryOp::Le));
                let tightened = match op {
                    BinaryOp::Gt | BinaryOp::Ge => tighten_lower(lower.take(), candidate),
                    BinaryOp::Lt | BinaryOp::Le => tighten_upper(upper.take(), candidate),
                    _ => continue,
                }?;
                match op {
                    BinaryOp::Gt | BinaryOp::Ge => lower = Some(tightened),
                    BinaryOp::Lt | BinaryOp::Le => upper = Some(tightened),
                    _ => unreachable!("guarded above"),
                }
                consumed.push(index);
            }
            binding_refs::PropertyPredicateShape::Between { low, high } => {
                let low = compatible_literal(low, kind)?.clone();
                let high = compatible_literal(high, kind)?.clone();
                return Some((
                    TypedIndexBounds::Range {
                        lo: low,
                        lo_inclusive: true,
                        hi: high,
                        hi_inclusive: true,
                    },
                    vec![index],
                ));
            }
            binding_refs::PropertyPredicateShape::InList(_) => {}
        }
    }

    if let Some(literal) = equality {
        return Some((TypedIndexBounds::Equality(literal), consumed));
    }
    match (lower, upper) {
        (Some((lo, lo_inclusive)), Some((hi, hi_inclusive))) => {
            // Contradictory bounds (lo > hi, or lo == hi with both exclusive) make the
            // range empty; refuse the rewrite so the executor evaluates the predicates
            // linearly and we don't paper over a possibly-buggy executor range path.
            if !range_satisfiable(&lo, lo_inclusive, &hi, hi_inclusive) {
                return None;
            }
            Some((
                TypedIndexBounds::Range {
                    lo,
                    lo_inclusive,
                    hi,
                    hi_inclusive,
                },
                consumed,
            ))
        }
        (Some((literal, false)), None) => Some((TypedIndexBounds::GreaterThan(literal), consumed)),
        (Some((literal, true)), None) => Some((TypedIndexBounds::GreaterEqual(literal), consumed)),
        (None, Some((literal, false))) => Some((TypedIndexBounds::LessThan(literal), consumed)),
        (None, Some((literal, true))) => Some((TypedIndexBounds::LessEqual(literal), consumed)),
        (None, None) => None,
    }
}

/// Combine two lower bounds, returning the tighter (higher) one, or `None`
/// if the literals can't be ordered (e.g., NaN). Inclusive/exclusive ties go
/// to the exclusive bound (it strictly excludes more rows).
fn tighten_lower(
    existing: Option<(Literal, bool)>,
    candidate: (Literal, bool),
) -> Option<(Literal, bool)> {
    let Some(existing) = existing else {
        return Some(candidate);
    };
    let ordering = compare_literals(&existing.0, &candidate.0)?;
    Some(match ordering {
        std::cmp::Ordering::Less => candidate,
        std::cmp::Ordering::Greater => existing,
        std::cmp::Ordering::Equal => {
            // Same literal; exclusive (false) wins because it filters one more value.
            (existing.0, existing.1 && candidate.1)
        }
    })
}

/// Combine two upper bounds, returning the tighter (lower) one.
fn tighten_upper(
    existing: Option<(Literal, bool)>,
    candidate: (Literal, bool),
) -> Option<(Literal, bool)> {
    let Some(existing) = existing else {
        return Some(candidate);
    };
    let ordering = compare_literals(&existing.0, &candidate.0)?;
    Some(match ordering {
        std::cmp::Ordering::Less => existing,
        std::cmp::Ordering::Greater => candidate,
        std::cmp::Ordering::Equal => (existing.0, existing.1 && candidate.1),
    })
}

/// Compare two literals of the same kind. Returns `None` when the literals
/// are non-orderable (e.g., NaN floats) or kinds differ.
fn compare_literals(a: &Literal, b: &Literal) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Literal::Integer(lhs, _), Literal::Integer(rhs, _)) => Some(lhs.cmp(rhs)),
        (Literal::Float(lhs, _), Literal::Float(rhs, _)) => lhs.partial_cmp(rhs),
        (Literal::String(lhs, _), Literal::String(rhs, _)) => Some(lhs.as_str().cmp(rhs.as_str())),
        (Literal::Bool(lhs, _), Literal::Bool(rhs, _)) => Some(lhs.cmp(rhs)),
        _ => None,
    }
}

/// Whether `[lo, hi]` (with the given inclusivities) contains at least one
/// value of the literal kind. Returns `true` for any orderable, non-empty
/// range; `false` for empty or non-orderable ranges (caller treats `false`
/// as "refuse rewrite").
fn range_satisfiable(lo: &Literal, lo_inclusive: bool, hi: &Literal, hi_inclusive: bool) -> bool {
    let Some(ordering) = compare_literals(lo, hi) else {
        return false;
    };
    match ordering {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => lo_inclusive && hi_inclusive,
    }
}

fn compatible_literal(value: &crate::ValueExpr, kind: crate::IndexKind) -> Option<&Literal> {
    let literal = binding_refs::literal(value)?;
    literal_matches_kind(literal, kind).then_some(literal)
}

fn binding_is_node(bindings: &[BindingDef], binding_id: crate::BindingId) -> bool {
    bindings.iter().any(|binding| {
        binding.binding == binding_id && binding.element == crate::BindingElement::Node
    })
}

fn remove_indices(predicates: &mut Vec<FilterPredicate>, indices: &[usize]) {
    let mut cursor = 0usize;
    predicates.retain(|_| {
        let remove = indices.binary_search(&cursor).is_ok();
        cursor += 1;
        !remove
    });
}
