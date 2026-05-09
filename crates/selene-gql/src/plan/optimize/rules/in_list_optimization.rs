//! Small `IN` list typed-index optimization.

use crate::plan::{
    BindingDef, ExecutionPlan, JoinTree, ScanAccess, ScanKind,
    optimize::{OptimizeContext, Rule, Transformed, binding_refs, walk},
};

use super::index_helpers::{literal_matches_kind, single_label};

const SMALL_IN_LIST_LIMIT: usize = 16;

/// Rewrite small property `IN` lists to bitmap-union access.
pub struct InListOptimization;

impl Rule for InListOptimization {
    fn name(&self) -> &'static str {
        "in_list_optimization"
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
    for index in 0..scan.property_predicates.len() {
        let pred = &scan.property_predicates[index];
        let Some(matched) = binding_refs::match_property_predicate(pred, bindings) else {
            continue;
        };
        if !binding_is_node(bindings, matched.binding) {
            continue;
        }
        let binding_refs::PropertyPredicateShape::InList(items) = matched.shape else {
            continue;
        };
        if items.is_empty() || items.len() > SMALL_IN_LIST_LIMIT {
            continue;
        }
        let Some(lookup) = catalog.typed_index(crate::IndexTarget::Node, label, matched.key) else {
            continue;
        };
        let mut keys = Vec::with_capacity(items.len());
        let mut all_match = true;
        for item in items {
            let Some(literal) = binding_refs::literal(item) else {
                all_match = false;
                break;
            };
            if !literal_matches_kind(literal, lookup.kind) {
                all_match = false;
                break;
            }
            keys.push(literal.clone());
        }
        if !all_match {
            continue;
        }
        scan.property_predicates.remove(index);
        scan.access = ScanAccess::BitmapUnion {
            handle: lookup.handle,
            keys,
        };
        return true;
    }
    false
}

fn binding_is_node(bindings: &[BindingDef], binding_id: crate::BindingId) -> bool {
    bindings.iter().any(|binding| {
        binding.binding == binding_id && binding.element == crate::BindingElement::Node
    })
}
