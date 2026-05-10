use crate::{
    LimitAmount,
    runtime::{BindingTable, ExecutorError},
};

pub(super) fn execute(
    offset: &LimitAmount,
    count: &LimitAmount,
    table: BindingTable,
) -> Result<BindingTable, ExecutorError> {
    let offset = literal_amount(offset)?;
    let count = literal_amount(count)?;
    let (schema, rows) = table.into_parts();
    let start = u64_to_bounded_usize(offset, rows.len());
    let end = start.saturating_add(u64_to_bounded_usize(count, rows.len() - start));
    Ok(BindingTable::new(schema, rows[start..end].to_vec()))
}

pub(super) fn literal_amount(amount: &LimitAmount) -> Result<u64, ExecutorError> {
    match amount {
        LimitAmount::Literal(value) => Ok(*value),
        LimitAmount::Parameter(_) => Err(ExecutorError::ImplementationDefined {
            detail: "parameterized LIMIT not implemented",
        }),
    }
}

pub(super) fn u64_to_bounded_usize(value: u64, upper_bound: usize) -> usize {
    usize::try_from(value)
        .unwrap_or(usize::MAX)
        .min(upper_bound)
}
