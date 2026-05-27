//! Composite index lookup rule.

use selene_core::IStr;

use crate::plan::{
    BindingDef, ExecutionPlan, JoinTree, ScanAccess, ScanKind,
    optimize::{OptimizeContext, Rule, Transformed, walk},
};

use super::index_helpers::{
    EqualityCandidate, compatible_value, equality_candidates, single_label,
};

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
        JoinTree::Expand { child, .. }
        | JoinTree::Questioned { child, .. }
        | JoinTree::Repeat { child, .. } => rewrite_tree(child, bindings, catalog),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            rewrite_tree(left, bindings, catalog) | rewrite_tree(right, bindings, catalog)
        }
        JoinTree::PathSearch { child, .. } | JoinTree::PathModeFilter { child, .. } => {
            rewrite_tree(child, bindings, catalog)
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => false,
        // Walk each per-label branch; downstream index rules apply
        // independently per branch (the rule that emits DisjunctiveScan
        // runs at slot 5 — before this rule at slot 6).
        JoinTree::DisjunctiveScan { branches, .. } => {
            branches.iter_mut().fold(false, |changed, branch| {
                rewrite_scan(branch, bindings, catalog) | changed
            })
        }
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
    // BRIEF-154 §B.2 F7: resolve each component value against its per-column
    // IndexKind. Literals get plan-time kind-checked via `compatible_value`'s
    // literal branch; typed-declared parameters get gated by
    // `gql_type_compatible_with_index_kind`. Any per-component failure
    // (mismatched literal kind, typed-incompatible parameter) aborts the
    // rewrite so the executor falls back to Linear evaluation.
    let mut keys = Vec::with_capacity(composite.properties.len());
    for (property, kind) in &composite.properties {
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.key == *property)
            .expect("matched property is present in candidate set");
        let Some(index_key) = compatible_value(candidate.value, *kind) else {
            return false;
        };
        keys.push((*property, index_key));
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
                    .map(|(property, _kind)| {
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

fn remove_indices(predicates: &mut Vec<crate::FilterPredicate>, indices: &[usize]) {
    let mut cursor = 0usize;
    predicates.retain(|_| {
        let remove = indices.binary_search(&cursor).is_ok();
        cursor += 1;
        !remove
    });
}
