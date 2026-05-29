//! Default optimizer rule registry.

use crate::plan::optimize::{
    Rule,
    rules::{
        AndSplitting, CompositeIndexLookup, ConstantFolding, DisjunctiveLabelExpansion,
        ExpandFilterPushdown, FilterPushdown, InListOptimization, IndexOrder, LabelScan,
        NodeFilterExtraction, PredicateReorder, RangeIndexScan, SymmetryBreaking, TopK, WcoJoin,
    },
};

/// Default structural optimizer rule set.
pub static DEFAULT_RULES: &[&'static dyn Rule] = &[
    &ConstantFolding,
    &AndSplitting,
    &FilterPushdown,
    &NodeFilterExtraction,
    &ExpandFilterPushdown,
    &DisjunctiveLabelExpansion,
    &CompositeIndexLookup,
    &InListOptimization,
    &RangeIndexScan,
    &IndexOrder,
    &LabelScan,
    &PredicateReorder,
    &WcoJoin,
    &SymmetryBreaking,
    &TopK,
];

/// Stable names of [`DEFAULT_RULES`] entries, in rule execution order.
pub static RULE_NAMES: &[&str] = &[
    "constant_folding",
    "and_splitting",
    "filter_pushdown",
    "node_filter_extraction",
    "expand_filter_pushdown",
    "disjunctive_label_expansion",
    "composite_index_lookup",
    "in_list_optimization",
    "range_index_scan",
    "index_order",
    "label_scan",
    "predicate_reorder",
    "wco_join",
    "symmetry_breaking",
    "top_k",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_names_match_default_rules() {
        let from_default = DEFAULT_RULES
            .iter()
            .map(|rule| rule.name())
            .collect::<Vec<_>>();
        assert_eq!(from_default, RULE_NAMES);
    }
}
