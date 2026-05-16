use std::cmp::Ordering;

use selene_core::Value;

use crate::{
    NullsPolicy, OrderDirection, OrderKey,
    runtime::{
        Binding, BindingTable, ExecutorError, TxContext, evaluator,
        value_compare::{self, NullSortOrder},
    },
};

pub(super) fn execute(
    keys: &[OrderKey],
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (schema, rows) = table.into_parts();
    let mut keyed_rows = rows
        .into_iter()
        .map(|row| {
            evaluate_key_tuple(keys, &row, &schema, ctx).map(|tuple| KeyedRow { tuple, row })
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Phase A deliberately ignores OrderKey.access. That planner hint is
    // reserved for the Phase B scan-order shortcut; executor sorting remains
    // explicit and stable here.
    keyed_rows.sort_by(|lhs, rhs| compare_key_tuples(&lhs.tuple, &rhs.tuple, keys));

    Ok(BindingTable::new(
        schema,
        keyed_rows.into_iter().map(|row| row.row).collect(),
    ))
}

struct KeyedRow {
    tuple: Vec<Value>,
    row: Binding,
}

pub(super) fn evaluate_key_tuple(
    keys: &[OrderKey],
    row: &Binding,
    schema: &crate::BindingTableSchema,
    ctx: &mut TxContext<'_, '_>,
) -> Result<Vec<Value>, ExecutorError> {
    keys.iter()
        .map(|key| evaluator::evaluate(&key.expr, row, schema, ctx))
        .collect()
}

pub(super) fn compare_key_tuples(lhs: &[Value], rhs: &[Value], keys: &[OrderKey]) -> Ordering {
    lhs.iter()
        .zip(rhs.iter())
        .zip(keys.iter())
        .map(|((lhs, rhs), key)| compare_key_value(lhs, rhs, key))
        .find(|ordering| *ordering != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

fn compare_key_value(lhs: &Value, rhs: &Value, key: &OrderKey) -> Ordering {
    let nulls = null_sort_order(key);
    let ordering = value_compare::compare_for_sort(lhs, rhs, nulls);
    match key.direction {
        OrderDirection::Asc => ordering,
        OrderDirection::Desc => ordering.reverse(),
    }
}

fn null_sort_order(key: &OrderKey) -> NullSortOrder {
    let desired = match key.nulls {
        Some(NullsPolicy::NullsFirst) => NullSortOrder::First,
        Some(NullsPolicy::NullsLast) => NullSortOrder::Last,
        None => match key.direction {
            OrderDirection::Asc => NullSortOrder::Last,
            OrderDirection::Desc => NullSortOrder::First,
        },
    };
    match (key.direction, desired) {
        (OrderDirection::Asc, order) => order,
        (OrderDirection::Desc, NullSortOrder::First) => NullSortOrder::Last,
        (OrderDirection::Desc, NullSortOrder::Last) => NullSortOrder::First,
    }
}
