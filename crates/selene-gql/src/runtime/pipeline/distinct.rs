use rustc_hash::FxHashSet;

use crate::runtime::{BindingTable, ExecutorError, TxContext};

use super::row_key;

pub(super) fn execute(
    table: BindingTable,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (schema, rows) = table.into_parts();
    let mut seen = FxHashSet::default();
    seen.reserve(rows.len());
    let mut output = Vec::with_capacity(rows.len());
    let mut rows_since_check = 0;
    for row in rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let key = row_key(&row);
        if seen.insert(key) {
            output.push(row);
        }
    }
    Ok(BindingTable::new(schema, output))
}
