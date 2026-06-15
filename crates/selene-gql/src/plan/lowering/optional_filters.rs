//! Optional MATCH filter partitioning.

use std::collections::BTreeSet;

use selene_core::DbString;

use crate::{
    analyze::{AnalyzedStatement, BindingDecl, BindingId},
    plan::FilterPredicate,
};

pub(super) fn split_optional_filters(
    filters: Vec<FilterPredicate>,
    left_names: &BTreeSet<DbString>,
    analyzed: &AnalyzedStatement,
) -> (Vec<FilterPredicate>, Vec<FilterPredicate>) {
    let mut right_filters = Vec::new();
    let mut global_filters = Vec::new();
    for filter in filters {
        if references_optional_binding(&filter, left_names, analyzed) {
            right_filters.push(filter);
        } else {
            global_filters.push(filter);
        }
    }
    (right_filters, global_filters)
}

fn references_optional_binding(
    filter: &FilterPredicate,
    left_names: &BTreeSet<DbString>,
    analyzed: &AnalyzedStatement,
) -> bool {
    filter.binding_refs.iter().any(|binding| {
        binding_name(*binding, analyzed).is_some_and(|name| !left_names.contains(&name))
    })
}

fn binding_name(binding: BindingId, analyzed: &AnalyzedStatement) -> Option<DbString> {
    analyzed
        .scopes
        .declarations()
        .iter()
        .find(|decl| decl.id() == binding)
        .map(BindingDecl::name)
}
