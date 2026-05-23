//! Set-composition pipeline operator.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    BindingTableSchema, ExecutionPlan, SetOp, SourceSpan,
    runtime::{
        Binding, BindingTable, DataExceptionSubclass, ExecutorError, TxContext, execute_plan,
        value_key::RuntimeEqKey,
    },
};

use super::{distinct, row_key};

pub(super) fn execute(
    op: SetOp,
    rhs: &ExecutionPlan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let rhs_table = execute_plan(rhs, ctx)?;
    execute_with_rhs(op, table, rhs_table, ctx)
}

pub(super) fn execute_with_rhs(
    op: SetOp,
    table: BindingTable,
    rhs_table: BindingTable,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    match op {
        SetOp::Union | SetOp::UnionAll => {
            assert_compatible_schemas("UNION", table.schema(), rhs_table.schema())?;
            let (schema, mut rows) = table.into_parts();
            ctx.check_cancellation()?;
            rows.extend(rhs_table.rows().iter().cloned());
            let combined = BindingTable::new(schema, rows);
            if matches!(op, SetOp::Union) {
                distinct::execute(combined, ctx)
            } else {
                Ok(combined)
            }
        }
        SetOp::Intersect | SetOp::IntersectAll | SetOp::Except | SetOp::ExceptAll => {
            assert_compatible_schemas(op_name(op), table.schema(), rhs_table.schema())?;
            execute_counted(op, table, &rhs_table, ctx)
        }
        SetOp::Otherwise => {
            let (schema, rows) = table.into_parts();
            assert_compatible_schemas("OTHERWISE", &schema, rhs_table.schema())?;
            if rows.is_empty() {
                assert_compatible_schemas("OTHERWISE", &schema, rhs_table.schema())?;
                let (_, rhs_rows) = rhs_table.into_parts();
                Ok(BindingTable::new(schema, rhs_rows))
            } else {
                Ok(BindingTable::new(schema, rows))
            }
        }
    }
}

fn execute_counted(
    op: SetOp,
    lhs: BindingTable,
    rhs: &BindingTable,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let mut rhs_counts = count_rows(rhs.rows(), ctx)?;
    let (schema, lhs_rows) = lhs.into_parts();
    let mut output = Vec::new();
    let mut seen = FxHashSet::default();
    let mut rows_since_check = 0;

    for row in lhs_rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        match op {
            SetOp::IntersectAll => {
                let key = row_key(&row);
                if let Some(count) = rhs_counts.get_mut(&key)
                    && *count > 0
                {
                    *count -= 1;
                    output.push(row);
                }
            }
            SetOp::Intersect => {
                let key = row_key(&row);
                if rhs_counts.contains_key(&key) && insert_seen(&mut seen, key, ctx)? {
                    output.push(row);
                }
            }
            SetOp::ExceptAll => {
                let key = row_key(&row);
                if let Some(count) = rhs_counts.get_mut(&key)
                    && *count > 0
                {
                    *count -= 1;
                    continue;
                }
                output.push(row);
            }
            SetOp::Except => {
                let key = row_key(&row);
                if !rhs_counts.contains_key(&key) && insert_seen(&mut seen, key, ctx)? {
                    output.push(row);
                }
            }
            SetOp::Union | SetOp::UnionAll | SetOp::Otherwise => unreachable!("set op prefiltered"),
        }
    }

    Ok(BindingTable::new(schema, output))
}

fn count_rows(
    rows: &[Binding],
    ctx: &TxContext<'_, '_>,
) -> Result<FxHashMap<RuntimeEqKey, usize>, ExecutorError> {
    let mut counts = FxHashMap::default();
    let mut rows_since_check = 0;
    for row in rows {
        ctx.check_cancellation_stride(&mut rows_since_check, 1)?;
        let key = row_key(row);
        if !counts.contains_key(&key) && counts.len() >= ctx.impl_defined_caps().set_op_key_cap() {
            return Err(set_op_key_cap_exceeded());
        }
        *counts.entry(key).or_insert(0) += 1;
    }
    Ok(counts)
}

fn insert_seen(
    seen: &mut FxHashSet<RuntimeEqKey>,
    key: RuntimeEqKey,
    ctx: &TxContext<'_, '_>,
) -> Result<bool, ExecutorError> {
    if seen.contains(&key) {
        return Ok(false);
    }
    if seen.len() >= ctx.impl_defined_caps().set_op_key_cap() {
        return Err(set_op_key_cap_exceeded());
    }
    Ok(seen.insert(key))
}

fn set_op_key_cap_exceeded() -> ExecutorError {
    ExecutorError::ProgramLimitExceeded {
        detail: "set-op key cap exceeded",
        span: SourceSpan::default(),
    }
}

fn assert_compatible_schemas(
    op_name: &'static str,
    lhs: &BindingTableSchema,
    rhs: &BindingTableSchema,
) -> Result<(), ExecutorError> {
    let lhs_len = lhs.columns.len();
    let rhs_len = rhs.columns.len();
    if lhs_len != rhs_len {
        return Err(ExecutorError::DataException {
            subclass: DataExceptionSubclass::InvalidValueType,
            message: format!(
                "{op_name} arms have differing column counts: lhs={lhs_len}, rhs={rhs_len}"
            ),
            span: SourceSpan::default(),
        });
    }
    Ok(())
}

fn op_name(op: SetOp) -> &'static str {
    match op {
        SetOp::Union | SetOp::UnionAll => "UNION",
        SetOp::Intersect | SetOp::IntersectAll => "INTERSECT",
        SetOp::Except | SetOp::ExceptAll => "EXCEPT",
        SetOp::Otherwise => "OTHERWISE",
    }
}
