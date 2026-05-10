use std::mem;

use selene_core::Value;

use crate::runtime::{Binding, BindingTable, value_compare};

pub(super) fn execute(table: BindingTable) -> BindingTable {
    let schema = table.schema().clone();
    let mut seen = Vec::new();
    for row in table.rows() {
        if !seen.iter().any(|existing| rows_equal(existing, row)) {
            seen.push(row.clone());
        }
    }
    BindingTable::new(schema, seen)
}

fn rows_equal(lhs: &Binding, rhs: &Binding) -> bool {
    lhs.values().len() == rhs.values().len()
        && lhs
            .values()
            .iter()
            .zip(rhs.values())
            .all(|(lhs, rhs)| values_equal(lhs, rhs))
}

fn values_equal(lhs: &Value, rhs: &Value) -> bool {
    if mem::discriminant(lhs) != mem::discriminant(rhs) {
        return false;
    }
    match (lhs, rhs) {
        (Value::Null, Value::Null) => true,
        _ => value_compare::equal_non_null(lhs, rhs),
    }
}
