use rustc_hash::FxHashSet;

use crate::runtime::{BindingTable, ExecutorError, TxContext, value_key::DistinctRowKey};

pub(super) fn execute(
    table: BindingTable,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (schema, rows) = table.into_parts();
    let mut seen = FxHashSet::default();
    let mut output = Vec::with_capacity(rows.len());
    let mut rows_since_check = 0;
    for row in rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let key = DistinctRowKey(row.values().to_vec());
        if seen.insert(key) {
            output.push(row);
        }
    }
    Ok(BindingTable::new(schema, output))
}
