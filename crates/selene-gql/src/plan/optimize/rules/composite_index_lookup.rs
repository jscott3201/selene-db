//! Composite index lookup rule.

use selene_core::IStr;

use crate::plan::{
    BindingDef, ExecutionPlan, FilterPredicate, JoinTree, ScanAccess, ScanKind,
    optimize::{OptimizeContext, Rule, Transformed, binding_refs, walk},
};

use super::index_helpers::single_label;

/// Rewrite multi-property equality predicates to composite index access.
pub struct CompositeIndexLookup;

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
    let mut candidates = equality_candidates(&scan.property_predicates, bindings);
    if candidates.len() < 2 {
        return false;
    }
    let properties = candidates
        .iter()
        .map(|candidate| candidate.key)
        .collect::<Vec<_>>();
    let Some(composite) = catalog.composite_index(crate::IndexTarget::Node, label, &properties)
    else {
        return false;
    };
    let mut keys = Vec::with_capacity(composite.properties.len());
    let mut consumed_indices = Vec::with_capacity(composite.properties.len());
    for property in &composite.properties {
        let Some(position) = candidates
            .iter()
            .position(|candidate| candidate.key == *property)
        else {
            return false;
        };
        let candidate = candidates.swap_remove(position);
        keys.push((*property, candidate.literal.clone()));
        consumed_indices.push(candidate.index);
    }
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
    predicates
        .iter()
        .enumerate()
        .filter_map(|(index, pred)| {
            let matched = binding_refs::match_property_predicate(pred, bindings)?;
            let binding_id = matched.binding;
            let binding_def = bindings
                .iter()
                .find(|binding| binding.binding == binding_id)?;
            if binding_def.element != crate::BindingElement::Node {
                return None;
            }
            let binding_literal = match matched.shape {
                binding_refs::PropertyPredicateShape::Equality(value) => {
                    binding_refs::literal(value)?
                }
                _ => return None,
            };
            Some(EqualityCandidate {
                index,
                key: matched.key,
                literal: binding_literal.clone(),
            })
        })
        .collect()
}

fn remove_indices(predicates: &mut Vec<FilterPredicate>, indices: &[usize]) {
    let mut cursor = 0usize;
    predicates.retain(|_| {
        let remove = indices.binary_search(&cursor).is_ok();
        cursor += 1;
        !remove
    });
}
