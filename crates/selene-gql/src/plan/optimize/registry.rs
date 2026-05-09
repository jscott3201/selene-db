//! Default optimizer rule registry.

use crate::plan::optimize::{
    Rule,
    rules::{AndSplitting, ConstantFolding, ExpandFilterPushdown, FilterPushdown, TopK},
};

/// Default structural optimizer rule set.
pub static DEFAULT_RULES: &[&'static dyn Rule] = &[
    &ConstantFolding,
    &AndSplitting,
    &FilterPushdown,
    &ExpandFilterPushdown,
    &TopK,
];
