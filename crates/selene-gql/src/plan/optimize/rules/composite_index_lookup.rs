//! Composite index lookup rule.

use selene_core::IStr;

use crate::plan::{
    BindingDef, ExecutionPlan, FilterPredicate, JoinTree, ScanAccess, ScanKind,
    optimize::{OptimizeContext, Rule, Transformed, binding_refs, walk},
};

use super::index_helpers::single_label;

/// Rewrite multi-property equality predicates to composite index access.
///
/// # Duplicate property-key handling
///
/// When multiple equality predicates share the same property key, the first
/// candidate per key is chosen for the index lookup and every other predicate
/// on the same key remains in the residual filter regardless of literal value.
/// Conflicting duplicates therefore produce an empty result via residual
/// rejection, while exact duplicates leave a redundant residual predicate for a
/// future cleanup rule.
pub struct CompositeIndexLookup;

const MAX_COMPOSITE_CANDIDATES: usize = 16;

impl Rule for CompositeIndexLookup {
    fn name(&self) -> &'static str {
        "composite_index_lookup"
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
        JoinTree::Expand { child, .. } => rewrite_tree(child, bindings, catalog),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            rewrite_tree(left, bindings, catalog) | rewrite_tree(right, bindings, catalog)
        }
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
    let candidates = equality_candidates(&scan.property_predicates, bindings);
    if candidates.len() < 2 {
        return false;
    }
    // Try the full equality candidate set first, then progressively smaller
    // subsets, so a query with predicates `(tenant, kind, status)` still uses
    // a `(tenant, kind)` composite index — `status` stays in
    // `property_predicates` as a residual filter.
    let Some((composite, consumed_indices)) = find_composite_match(&candidates, label, catalog)
    else {
        return false;
    };
    let mut keys = Vec::with_capacity(composite.properties.len());
    for property in &composite.properties {
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.key == *property)
            .expect("matched property is present in candidate set");
        keys.push((*property, candidate.literal.clone()));
    }
    let mut consumed_indices = consumed_indices;
    consumed_indices.sort_unstable();
    consumed_indices.dedup();
    remove_indices(&mut scan.property_predicates, &consumed_indices);
    scan.access = ScanAccess::CompositeLookup {
        handle: composite.handle,
        properties: composite.properties,
        keys,
    };
    true
}

/// Try the full equality candidate set first, then every smaller subset
/// (down to size 2), preferring larger subsets. Returns the first matching
/// composite index along with the candidate-`property_predicates` indices it
/// consumed.
fn find_composite_match(
    candidates: &[EqualityCandidate],
    label: IStr,
    catalog: &dyn crate::IndexCatalog,
) -> Option<(crate::CompositeIndexHandle, Vec<usize>)> {
    let n = candidates.len();
    if n > MAX_COMPOSITE_CANDIDATES {
        return None;
    }
    // From largest subset down to size 2; within each size, iterate masks in
    // ascending order for deterministic plans when multiple indexes match.
    for size in (2..=n).rev() {
        let mut mask = (1u64 << size) - 1;
        while mask < (1u64 << n) {
            let subset_keys: Vec<IStr> = (0..n)
                .filter(|i| (mask >> i) & 1 == 1)
                .map(|i| candidates[i].key)
                .collect();
            if let Some(composite) =
                catalog.composite_index(crate::IndexTarget::Node, label, &subset_keys)
            {
                let consumed: Vec<usize> = composite
                    .properties
                    .iter()
                    .map(|property| {
                        candidates
                            .iter()
                            .find(|candidate| candidate.key == *property)
                            .map(|candidate| candidate.index)
                            .expect("matched property in candidate set")
                    })
                    .collect();
                return Some((composite, consumed));
            }
            // Gosper's hack: next mask with the same popcount, lexicographically.
            let c = mask & mask.wrapping_neg();
            let r = mask + c;
            mask = (((r ^ mask) >> 2) / c) | r;
        }
    }
    None
}

#[derive(Clone)]
struct EqualityCandidate {
    index: usize,
    key: IStr,
    literal: crate::Literal,
}

fn equality_candidates(
    predicates: &[FilterPredicate],
    bindings: &[BindingDef],
) -> Vec<EqualityCandidate> {
    let mut candidates = Vec::new();
    for (index, pred) in predicates.iter().enumerate() {
        let Some(matched) = binding_refs::match_property_predicate(pred, bindings) else {
            continue;
        };
        let binding_id = matched.binding;
        let Some(binding_def) = bindings
            .iter()
            .find(|binding| binding.binding == binding_id)
        else {
            continue;
        };
        if binding_def.element != crate::BindingElement::Node {
            continue;
        }
        if candidates
            .iter()
            .any(|candidate: &EqualityCandidate| candidate.key == matched.key)
        {
            continue;
        }
        let binding_literal = match matched.shape {
            binding_refs::PropertyPredicateShape::Equality(value) => {
                let Some(literal) = binding_refs::literal(value) else {
                    continue;
                };
                literal
            }
            _ => continue,
        };
        candidates.push(EqualityCandidate {
            index,
            key: matched.key,
            literal: binding_literal.clone(),
        });
    }
    candidates
}

fn remove_indices(predicates: &mut Vec<FilterPredicate>, indices: &[usize]) {
    let mut cursor = 0usize;
    predicates.retain(|_| {
        let remove = indices.binary_search(&cursor).is_ok();
        cursor += 1;
        !remove
    });
}
