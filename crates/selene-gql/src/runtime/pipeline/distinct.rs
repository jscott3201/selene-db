use rustc_hash::FxHashSet;

use crate::runtime::{BindingTable, value_key::DistinctRowKey};

pub(super) fn execute(table: BindingTable) -> BindingTable {
    let (schema, rows) = table.into_parts();
    let mut seen = FxHashSet::default();
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        let key = DistinctRowKey(row.values().to_vec());
        if seen.insert(key) {
            output.push(row);
        }
    }
    BindingTable::new(schema, output)
}
