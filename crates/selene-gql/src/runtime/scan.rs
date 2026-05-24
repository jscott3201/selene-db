//! Single-scan pattern executor.

use std::collections::BTreeSet;
use std::ops::Bound::{Excluded, Included, Unbounded};

use selene_core::{EdgeId, IStr, LabelSet, NodeId, Value};

use crate::{
    FilterPredicate, FilterPredicateKind, LabelExpr, Literal, NodeOrEdgeScan, PatternPlan,
    ScanAccess, ScanKind, TypedIndexBounds,
    runtime::{Binding, BindingTableSchema, ExecutorError},
};

use super::{EvalCtx, evaluator, pattern, value_compare};

/// Execute one `JoinTree::Scan` against the transaction snapshot.
pub(crate) fn scan_pattern(
    scan: &NodeOrEdgeScan,
    pattern: &PatternPlan,
    schema: &BindingTableSchema,
    seed: Option<&Binding>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    Ok(scan_entities(scan, pattern, schema, seed, ctx)?
        .into_iter()
        .map(|(_, binding)| binding)
        .collect())
}

pub(crate) fn scan_entities(
    scan: &NodeOrEdgeScan,
    pattern: &PatternPlan,
    schema: &BindingTableSchema,
    seed: Option<&Binding>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<(Value, Binding)>, ExecutorError> {
    let mut rows = Vec::new();
    for row in candidate_rows(scan, ctx) {
        if !label_matches_scan(scan, row, ctx) {
            continue;
        }
        let entity = entity_value(scan.kind, row);
        let Some(binding) = binding_for_scan(scan, pattern, schema, seed, entity.clone())? else {
            continue;
        };
        if predicates_pass(scan, pattern, &binding, schema, &entity, ctx)? {
            rows.push((entity, binding));
        }
    }
    Ok(rows)
}

fn binding_for_scan(
    scan: &NodeOrEdgeScan,
    pattern: &PatternPlan,
    schema: &BindingTableSchema,
    seed: Option<&Binding>,
    entity: Value,
) -> Result<Option<Binding>, ExecutorError> {
    let mut values = seed
        .map(|row| row.values().to_vec())
        .unwrap_or_else(|| vec![Value::Null; schema.columns.len()]);
    values.resize(schema.columns.len(), Value::Null);
    if !pattern::set_binding_value(&mut values, pattern, schema, scan.binding, entity.clone())? {
        return Ok(None);
    }
    if !pattern::set_hidden_value(&mut values, schema, scan.hidden_binding, entity)? {
        return Ok(None);
    }
    Ok(Some(Binding::new(values)))
}

fn candidate_rows(scan: &NodeOrEdgeScan, ctx: &EvalCtx<'_, '_, '_, '_>) -> Vec<u32> {
    match &scan.access {
        ScanAccess::Linear => linear_rows(scan.kind, ctx),
        ScanAccess::LabelIndex { .. } => label_index_rows(scan, ctx),
        ScanAccess::TypedIndexRange {
            property, bounds, ..
        } => typed_index_rows(scan, *property, bounds, ctx),
        ScanAccess::BitmapUnion { property, keys, .. } => {
            bitmap_union_rows(scan, *property, keys, ctx)
        }
        ScanAccess::CompositeLookup { keys, .. } => composite_lookup_rows(scan, keys, ctx),
    }
}

fn linear_rows(kind: ScanKind, ctx: &EvalCtx<'_, '_, '_, '_>) -> Vec<u32> {
    match kind {
        ScanKind::Node => ctx.tx.snapshot().node_store.alive.iter().collect(),
        ScanKind::Edge => ctx.tx.snapshot().edge_store.alive.iter().collect(),
    }
}

fn label_index_rows(scan: &NodeOrEdgeScan, ctx: &EvalCtx<'_, '_, '_, '_>) -> Vec<u32> {
    let Some(label) = single_label(&scan.label_predicate) else {
        return linear_rows(scan.kind, ctx);
    };
    match scan.kind {
        ScanKind::Node => ctx
            .tx
            .snapshot()
            .nodes_with_label(&label)
            .map(|rows| rows.iter().collect())
            .unwrap_or_default(),
        ScanKind::Edge => ctx
            .tx
            .snapshot()
            .edges_with_label(&label)
            .map(|rows| rows.iter().collect())
            .unwrap_or_default(),
    }
}

fn typed_index_rows(
    scan: &NodeOrEdgeScan,
    property: IStr,
    bounds: &TypedIndexBounds,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<u32> {
    if scan.kind != ScanKind::Node {
        return linear_rows_filtered_by_bounds(scan, property, bounds, ctx);
    }
    let Some(label) = single_label(&scan.label_predicate) else {
        return linear_rows_filtered_by_bounds(scan, property, bounds, ctx);
    };
    let value = |literal: &Literal| literal_value(literal);
    let indexed_rows = match bounds {
        TypedIndexBounds::Equality(literal) => ctx
            .tx
            .snapshot()
            .nodes_with_property_eq(&label, &property, &value(literal))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        TypedIndexBounds::GreaterThan(literal) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Excluded(value(literal)), Unbounded))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        TypedIndexBounds::GreaterEqual(literal) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Included(value(literal)), Unbounded))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        TypedIndexBounds::LessThan(literal) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Unbounded, Excluded(value(literal))))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        TypedIndexBounds::LessEqual(literal) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Unbounded, Included(value(literal))))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        TypedIndexBounds::Range {
            lo,
            lo_inclusive,
            hi,
            hi_inclusive,
        } => {
            let lo = if *lo_inclusive {
                Included(value(lo))
            } else {
                Excluded(value(lo))
            };
            let hi = if *hi_inclusive {
                Included(value(hi))
            } else {
                Excluded(value(hi))
            };
            ctx.tx
                .snapshot()
                .nodes_with_property_range(&label, &property, (lo, hi))
                .map(|rows| rows.iter().collect::<Vec<_>>())
        }
    };
    indexed_rows.unwrap_or_else(|| linear_rows_filtered_by_bounds(scan, property, bounds, ctx))
}

fn bitmap_union_rows(
    scan: &NodeOrEdgeScan,
    property: IStr,
    keys: &[Literal],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<u32> {
    union_property_eq(scan, property, keys, ctx)
        .into_iter()
        .collect()
}

fn union_property_eq(
    scan: &NodeOrEdgeScan,
    property: IStr,
    keys: &[Literal],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> BTreeSet<u32> {
    if scan.kind != ScanKind::Node {
        return linear_rows(scan.kind, ctx)
            .into_iter()
            .filter(|row| property_matches_any(scan.kind, *row, property, keys, ctx))
            .collect();
    }
    let Some(label) = single_label(&scan.label_predicate) else {
        return linear_rows(scan.kind, ctx)
            .into_iter()
            .filter(|row| property_matches_any(scan.kind, *row, property, keys, ctx))
            .collect();
    };
    let mut rows = BTreeSet::new();
    let mut used_index = false;
    for key in keys {
        if let Some(matches) =
            ctx.tx
                .snapshot()
                .nodes_with_property_eq(&label, &property, &literal_value(key))
        {
            used_index = true;
            rows.extend(matches.iter());
        }
    }
    if used_index {
        rows
    } else {
        linear_rows(scan.kind, ctx)
            .into_iter()
            .filter(|row| property_matches_any(scan.kind, *row, property, keys, ctx))
            .collect()
    }
}

fn composite_lookup_rows(
    scan: &NodeOrEdgeScan,
    keys: &[(IStr, Literal)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<u32> {
    linear_rows(scan.kind, ctx)
        .into_iter()
        .filter(|row| {
            keys.iter().all(|(property, literal)| {
                property_value(scan.kind, *row, *property, ctx)
                    .is_some_and(|value| value_eq_non_null(value, &literal_value(literal)))
            })
        })
        .collect()
}

fn linear_rows_filtered_by_bounds(
    scan: &NodeOrEdgeScan,
    property: IStr,
    bounds: &TypedIndexBounds,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<u32> {
    linear_rows(scan.kind, ctx)
        .into_iter()
        .filter(|row| {
            property_value(scan.kind, *row, property, ctx)
                .is_some_and(|value| value_matches_bounds(value, bounds))
        })
        .collect()
}

pub(crate) fn predicates_pass(
    scan: &NodeOrEdgeScan,
    pattern: &PatternPlan,
    binding: &Binding,
    schema: &BindingTableSchema,
    entity: &Value,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    for predicate in &scan.property_predicates {
        if !predicate_passes(predicate, pattern, binding, schema, entity, ctx)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn predicate_passes(
    predicate: &FilterPredicate,
    pattern: &PatternPlan,
    binding: &Binding,
    schema: &BindingTableSchema,
    entity: &Value,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    if predicate.index_consumed {
        return Ok(true);
    }
    match &predicate.kind {
        FilterPredicateKind::Expression => {
            let value = evaluator::evaluate(&predicate.expr, binding, schema, ctx)?;
            Ok(matches!(value, Value::Bool(true)))
        }
        FilterPredicateKind::PropertyEquals {
            binding: property_binding,
            key,
        } => {
            let target = property_binding
                .and_then(|binding_id| value_for_binding(pattern, binding_id, binding, schema))
                .unwrap_or_else(|| entity.clone());
            let property = match &target {
                Value::NodeRef(id) => ctx
                    .tx
                    .snapshot()
                    .node_properties(*id)
                    .and_then(|properties| properties.get(key))
                    .cloned(),
                Value::EdgeRef(id) => ctx
                    .tx
                    .snapshot()
                    .edge_properties(*id)
                    .and_then(|properties| properties.get(key))
                    .cloned(),
                Value::Null => None,
                _ => None,
            }
            .unwrap_or(Value::Null);
            let expected = evaluator::evaluate(&predicate.expr, binding, schema, ctx)?;
            Ok(value_eq_non_null(&property, &expected))
        }
    }
}

pub(crate) fn value_for_binding(
    pattern: &PatternPlan,
    binding_id: crate::BindingId,
    binding: &Binding,
    schema: &BindingTableSchema,
) -> Option<Value> {
    let binding_def = pattern
        .bindings
        .iter()
        .find(|candidate| candidate.binding == binding_id)?;
    let index = schema
        .columns
        .iter()
        .position(|column| column.name == Some(binding_def.name))?;
    binding.get(index).cloned()
}

fn label_matches_scan(scan: &NodeOrEdgeScan, row: u32, ctx: &EvalCtx<'_, '_, '_, '_>) -> bool {
    let Some(label_expr) = &scan.label_predicate else {
        return true;
    };
    match scan.kind {
        ScanKind::Node => {
            let id = NodeId::new(u64::from(row) + 1);
            ctx.tx
                .snapshot()
                .node_labels(id)
                .is_some_and(|labels| label_matches_node(label_expr, labels))
        }
        ScanKind::Edge => {
            let id = EdgeId::new(u64::from(row) + 1);
            ctx.tx
                .snapshot()
                .edge_label(id)
                .is_some_and(|label| label_matches_edge(label_expr, *label))
        }
    }
}

pub(crate) fn label_matches_node(expr: &LabelExpr, labels: &LabelSet) -> bool {
    match expr {
        LabelExpr::Single(label) => labels.contains(label),
        LabelExpr::Conjunction(parts) => parts.iter().all(|part| label_matches_node(part, labels)),
        LabelExpr::Disjunction(parts) => parts.iter().any(|part| label_matches_node(part, labels)),
        LabelExpr::Negation(part) => !label_matches_node(part, labels),
        LabelExpr::Wildcard => true,
    }
}

pub(crate) fn label_matches_edge(expr: &LabelExpr, label: IStr) -> bool {
    match expr {
        LabelExpr::Single(expected) => *expected == label,
        LabelExpr::Conjunction(parts) => parts.iter().all(|part| label_matches_edge(part, label)),
        LabelExpr::Disjunction(parts) => parts.iter().any(|part| label_matches_edge(part, label)),
        LabelExpr::Negation(part) => !label_matches_edge(part, label),
        LabelExpr::Wildcard => true,
    }
}

fn single_label(label: &Option<LabelExpr>) -> Option<IStr> {
    match label {
        Some(LabelExpr::Single(label)) => Some(*label),
        _ => None,
    }
}

fn entity_value(kind: ScanKind, row: u32) -> Value {
    match kind {
        ScanKind::Node => Value::NodeRef(NodeId::new(u64::from(row) + 1)),
        ScanKind::Edge => Value::EdgeRef(EdgeId::new(u64::from(row) + 1)),
    }
}

fn property_matches_any(
    kind: ScanKind,
    row: u32,
    property: IStr,
    keys: &[Literal],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> bool {
    property_value(kind, row, property, ctx).is_some_and(|value| {
        keys.iter()
            .any(|key| value_eq_non_null(value, &literal_value(key)))
    })
}

fn property_value<'a>(
    kind: ScanKind,
    row: u32,
    property: IStr,
    ctx: &'a EvalCtx<'_, '_, '_, '_>,
) -> Option<&'a Value> {
    match kind {
        ScanKind::Node => ctx
            .tx
            .snapshot()
            .node_properties(NodeId::new(u64::from(row) + 1))
            .and_then(|properties| properties.get(&property)),
        ScanKind::Edge => ctx
            .tx
            .snapshot()
            .edge_properties(EdgeId::new(u64::from(row) + 1))
            .and_then(|properties| properties.get(&property)),
    }
}

fn value_matches_bounds(value: &Value, bounds: &TypedIndexBounds) -> bool {
    match bounds {
        TypedIndexBounds::Equality(literal) => value_eq_non_null(value, &literal_value(literal)),
        TypedIndexBounds::GreaterThan(literal) => {
            value_compare::compare_non_null(value, &literal_value(literal))
                == Some(std::cmp::Ordering::Greater)
        }
        TypedIndexBounds::GreaterEqual(literal) => matches!(
            value_compare::compare_non_null(value, &literal_value(literal)),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
        TypedIndexBounds::LessThan(literal) => {
            value_compare::compare_non_null(value, &literal_value(literal))
                == Some(std::cmp::Ordering::Less)
        }
        TypedIndexBounds::LessEqual(literal) => matches!(
            value_compare::compare_non_null(value, &literal_value(literal)),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        TypedIndexBounds::Range {
            lo,
            lo_inclusive,
            hi,
            hi_inclusive,
        } => {
            let Some(lo_order) = value_compare::compare_non_null(value, &literal_value(lo)) else {
                return false;
            };
            let Some(hi_order) = value_compare::compare_non_null(value, &literal_value(hi)) else {
                return false;
            };
            let lo_ok = if *lo_inclusive {
                matches!(
                    lo_order,
                    std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
                )
            } else {
                lo_order == std::cmp::Ordering::Greater
            };
            let hi_ok = if *hi_inclusive {
                matches!(
                    hi_order,
                    std::cmp::Ordering::Less | std::cmp::Ordering::Equal
                )
            } else {
                hi_order == std::cmp::Ordering::Less
            };
            lo_ok && hi_ok
        }
    }
}

fn literal_value(literal: &Literal) -> Value {
    match literal {
        Literal::Bool(value, _) => Value::Bool(*value),
        Literal::Integer(value, _) => Value::Int(*value),
        Literal::Float(value, _) => Value::Float(*value),
        Literal::String(value, _) => Value::String(*value),
        Literal::Uuid(value, _) => Value::Uuid(*value),
        Literal::Null(_) => Value::Null,
    }
}

fn value_eq_non_null(lhs: &Value, rhs: &Value) -> bool {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return false;
    }
    value_compare::equal_non_null(lhs, rhs)
}
