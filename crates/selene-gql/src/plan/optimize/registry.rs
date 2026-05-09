//! Default optimizer rule registry.

use crate::plan::optimize::{
    Rule,
    rules::{
        AndSplitting, CompositeIndexLookup, ConstantFolding, ExpandFilterPushdown, FilterPushdown,
        InListOptimization, IndexOrder, NodeFilterExtraction, PredicateReorder, RangeIndexScan,
        SymmetryBreaking, TopK, WcoJoin,
    },
};

/// Default structural optimizer rule set.
pub static DEFAULT_RULES: &[&'static dyn Rule] = &[
    &ConstantFolding,
    &AndSplitting,
    &FilterPushdown,
    &NodeFilterExtraction,
    &ExpandFilterPushdown,
    &CompositeIndexLookup,
    &InListOptimization,
    &RangeIndexScan,
    &IndexOrder,
    &PredicateReorder,
    &WcoJoin,
    &SymmetryBreaking,
    &TopK,
];
