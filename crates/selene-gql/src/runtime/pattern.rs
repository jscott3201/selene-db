//! Pattern join-tree executor.

use selene_core::{IStr, Value};

use crate::{
    BindingElement, BindingId, BindingTableColumn, BindingTableSchema, FilterPredicate, JoinTree,
    PatternPlan,
    runtime::{Binding, BindingTable, ExecutorError, TxContext},
};

use super::{evaluator, expand, hash_join, outer, scan, subplan, value_compare, wco};

/// Execute a pattern plan and produce its initial binding table.
pub fn execute_pattern(
    pattern: &PatternPlan,
    ctx: &TxContext<'_>,
) -> Result<BindingTable, ExecutorError> {
    let schema = schema_for_pattern(pattern);
    let env = WalkContext {
        pattern,
        schema: &schema,
        seed: None,
        ctx,
    };
    let rows = walk_join_tree(&pattern.join_tree, env)?
        .into_iter()
        .filter_map(
            |row| match pattern_filters_pass(pattern, &row, &schema, ctx) {
                Ok(true) => Some(Ok(row)),
                Ok(false) => None,
                Err(err) => Some(Err(err)),
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BindingTable::new(schema, rows))
}

#[derive(Clone, Copy)]
pub(crate) struct WalkContext<'a, 'seed, 'ctx> {
    pub(crate) pattern: &'a PatternPlan,
    pub(crate) schema: &'a BindingTableSchema,
    pub(crate) seed: Option<&'seed Binding>,
    pub(crate) ctx: &'a TxContext<'ctx>,
}

pub(crate) fn walk_join_tree(
    tree: &JoinTree,
    env: WalkContext<'_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    match tree {
        JoinTree::Scan(scan_node) => {
            scan::scan_pattern(scan_node, env.pattern, env.schema, env.seed, env.ctx)
        }
        JoinTree::Expand {
            child,
            edge,
            direction,
        } => expand::execute(child, edge, *direction, env),
        JoinTree::HashJoin {
            left,
            right,
            key,
            build_side,
        } => hash_join::execute(left, right, key, *build_side, env),
        JoinTree::Outer { left, right, key } => outer::execute(left, right, key, env),
        JoinTree::WorstCaseOptimal { intersection, .. } => wco::execute_phase_a(intersection, env),
        JoinTree::Subplan(plan) => subplan::execute(plan, env.schema, env.seed, env.ctx),
    }
}

pub(crate) fn schema_for_pattern(pattern: &PatternPlan) -> BindingTableSchema {
    BindingTableSchema {
        columns: pattern
            .bindings
            .iter()
            .filter(|binding| {
                matches!(
                    binding.element,
                    BindingElement::Node | BindingElement::Edge | BindingElement::Path
                )
            })
            .map(|binding| BindingTableColumn {
                name: Some(binding.name),
                ty: binding.ty.clone(),
            })
            .collect(),
    }
}

pub(crate) fn binding_index(
    pattern: &PatternPlan,
    schema: &BindingTableSchema,
    binding_id: BindingId,
) -> Option<usize> {
    let binding = pattern
        .bindings
        .iter()
        .find(|candidate| candidate.binding == binding_id)?;
    column_index(schema, binding.name)
}

pub(crate) fn column_index(schema: &BindingTableSchema, name: IStr) -> Option<usize> {
    schema
        .columns
        .iter()
        .position(|column| column.name == Some(name))
}

pub(crate) fn set_binding_value(
    values: &mut [Value],
    pattern: &PatternPlan,
    schema: &BindingTableSchema,
    binding_id: Option<BindingId>,
    value: Value,
) -> Result<bool, ExecutorError> {
    let Some(binding_id) = binding_id else {
        return Ok(true);
    };
    let Some(index) = binding_index(pattern, schema, binding_id) else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "binding column missing from pattern schema",
        });
    };
    if !matches!(values[index], Value::Null)
        && !value_compare::equal_non_null(&values[index], &value)
    {
        return Ok(false);
    }
    values[index] = value;
    Ok(true)
}

pub(crate) fn merge_rows(left: &Binding, right: &Binding, schema: &BindingTableSchema) -> Binding {
    let mut values = Vec::with_capacity(schema.columns.len());
    for index in 0..schema.columns.len() {
        let left_value = left.get(index).cloned().unwrap_or(Value::Null);
        if matches!(left_value, Value::Null) {
            values.push(right.get(index).cloned().unwrap_or(Value::Null));
        } else {
            values.push(left_value);
        }
    }
    Binding::new(values)
}

pub(crate) fn key_values(
    row: &Binding,
    schema: &BindingTableSchema,
    key: &[IStr],
) -> Result<Option<Vec<Value>>, ExecutorError> {
    let mut values = Vec::with_capacity(key.len());
    for name in key {
        let Some(index) = column_index(schema, *name) else {
            return Err(ExecutorError::ImplementationDefined {
                detail: "join key column missing from pattern schema",
            });
        };
        let value = row.get(index).cloned().unwrap_or(Value::Null);
        if matches!(value, Value::Null) {
            return Ok(None);
        }
        values.push(value);
    }
    Ok(Some(values))
}

pub(crate) fn key_values_equal(lhs: &[Value], rhs: &[Value]) -> bool {
    lhs.len() == rhs.len()
        && lhs
            .iter()
            .zip(rhs)
            .all(|(lhs, rhs)| value_compare::equal_non_null(lhs, rhs))
}

pub(crate) fn rows_match_on_key(
    left: &Binding,
    right: &Binding,
    schema: &BindingTableSchema,
    key: &[IStr],
) -> Result<bool, ExecutorError> {
    let Some(left_key) = key_values(left, schema, key)? else {
        return Ok(false);
    };
    let Some(right_key) = key_values(right, schema, key)? else {
        return Ok(false);
    };
    Ok(key_values_equal(&left_key, &right_key))
}

pub(crate) fn project_row_to_schema(
    row: &Binding,
    source_schema: &BindingTableSchema,
    target_schema: &BindingTableSchema,
    seed: Option<&Binding>,
) -> Binding {
    let mut values = seed
        .map(|row| row.values().to_vec())
        .unwrap_or_else(|| vec![Value::Null; target_schema.columns.len()]);
    values.resize(target_schema.columns.len(), Value::Null);
    for (target_index, target_column) in target_schema.columns.iter().enumerate() {
        let Some(name) = target_column.name else {
            continue;
        };
        let Some(source_index) = column_index(source_schema, name) else {
            continue;
        };
        values[target_index] = row.get(source_index).cloned().unwrap_or(Value::Null);
    }
    Binding::new(values)
}

fn pattern_filters_pass(
    pattern: &PatternPlan,
    row: &Binding,
    schema: &BindingTableSchema,
    ctx: &TxContext<'_>,
) -> Result<bool, ExecutorError> {
    for predicate in &pattern.filters {
        if !filter_predicate_passes(predicate, pattern, row, schema, ctx)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn filter_predicate_passes(
    predicate: &FilterPredicate,
    pattern: &PatternPlan,
    row: &Binding,
    schema: &BindingTableSchema,
    ctx: &TxContext<'_>,
) -> Result<bool, ExecutorError> {
    if predicate.index_consumed {
        return Ok(true);
    }
    match predicate.kind {
        crate::FilterPredicateKind::Expression => {
            let value = evaluator::evaluate(&predicate.expr, row, schema, ctx)?;
            Ok(matches!(value, Value::Bool(true)))
        }
        crate::FilterPredicateKind::PropertyEquals { .. } => {
            scan::predicate_passes(predicate, pattern, row, schema, &Value::Null, ctx)
        }
    }
}
