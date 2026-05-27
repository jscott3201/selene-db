//! Single-scan pattern executor.

use std::collections::BTreeSet;
use std::ops::Bound::{Excluded, Included, Unbounded};

use selene_core::{EdgeId, IStr, LabelSet, NodeId, Value};

use crate::{
    FilterPredicate, FilterPredicateKind, IndexKey, LabelExpr, Literal, NodeOrEdgeScan,
    PatternPlan, ScanAccess, ScanKind, TypedIndexBounds,
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
        ScanAccess::CompositeLookup {
            properties, keys, ..
        } => composite_lookup_rows(scan, properties, keys, ctx),
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
    // Pre-Commit-4 bridge: optimizer rules never emit `IndexKey::Parameter`
    // yet, so every key reaches this site as `Literal`. Commit 4 will replace
    // these `literal_for_pre_param_path()` calls with `resolve_index_key`
    // against `&EvalCtx` parameters.
    let value = |key: &IndexKey| literal_value(key.literal_for_pre_param_path());
    let indexed_rows = match bounds {
        TypedIndexBounds::Equality(key) => ctx
            .tx
            .snapshot()
            .nodes_with_property_eq(&label, &property, &value(key))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        TypedIndexBounds::GreaterThan(key) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Excluded(value(key)), Unbounded))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        TypedIndexBounds::GreaterEqual(key) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Included(value(key)), Unbounded))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        TypedIndexBounds::LessThan(key) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Unbounded, Excluded(value(key))))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        TypedIndexBounds::LessEqual(key) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Unbounded, Included(value(key))))
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
    keys: &[IndexKey],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<u32> {
    union_property_eq(scan, property, keys, ctx)
        .into_iter()
        .collect()
}

fn union_property_eq(
    scan: &NodeOrEdgeScan,
    property: IStr,
    keys: &[IndexKey],
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
        let literal = key.literal_for_pre_param_path();
        if let Some(matches) =
            ctx.tx
                .snapshot()
                .nodes_with_property_eq(&label, &property, &literal_value(literal))
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
    properties: &[IStr],
    keys: &[(IStr, IndexKey)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<u32> {
    if scan.kind != ScanKind::Node {
        return linear_rows_filtered_by_composite(scan, keys, ctx);
    }
    let Some(label) = single_label(&scan.label_predicate) else {
        return linear_rows_filtered_by_composite(scan, keys, ctx);
    };
    let Some(values) = composite_lookup_values(properties, keys) else {
        return linear_rows_filtered_by_composite(scan, keys, ctx);
    };
    if let Some(index) = ctx
        .tx
        .snapshot()
        .composite_property_index_for(&label, properties)
    {
        let refs = values.iter().collect::<Vec<_>>();
        // Read-path MUST NOT admit new strings into the IStr pool (BRIEF-153);
        // an unpoolable `Value::ExternalString` component proves no indexed
        // row could match, so render as empty without admission.
        match index.key_from_values_lookup(&refs) {
            Ok(Some(key)) => {
                return index
                    .lookup_key(&key)
                    .map(|bitmap| bitmap.iter().collect())
                    .unwrap_or_default();
            }
            Ok(None) => return Vec::new(),
            Err(_) => {}
        }
    }
    linear_rows_filtered_by_composite(scan, keys, ctx)
}

fn composite_lookup_values(properties: &[IStr], keys: &[(IStr, IndexKey)]) -> Option<Vec<Value>> {
    properties
        .iter()
        .map(|property| {
            keys.iter()
                .find(|(key, _)| key == property)
                .map(|(_, key)| literal_value(key.literal_for_pre_param_path()))
        })
        .collect()
}

fn linear_rows_filtered_by_composite(
    scan: &NodeOrEdgeScan,
    keys: &[(IStr, IndexKey)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<u32> {
    linear_rows(scan.kind, ctx)
        .into_iter()
        .filter(|row| {
            keys.iter().all(|(property, key)| {
                let literal = key.literal_for_pre_param_path();
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
    keys: &[IndexKey],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> bool {
    property_value(kind, row, property, ctx).is_some_and(|value| {
        keys.iter().any(|key| {
            let literal = key.literal_for_pre_param_path();
            value_eq_non_null(value, &literal_value(literal))
        })
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
    // Pre-Commit-4 bridge: optimizer rules emit only `IndexKey::Literal` for
    // now. Commit 4 introduces Q12's scan-entry pre-resolution that hands this
    // function a `&[Value]` aligned with bounds; today we destructure straight
    // through `literal_for_pre_param_path()`.
    let lit = |key: &IndexKey| literal_value(key.literal_for_pre_param_path());
    match bounds {
        TypedIndexBounds::Equality(key) => value_eq_non_null(value, &lit(key)),
        TypedIndexBounds::GreaterThan(key) => {
            value_compare::compare_non_null(value, &lit(key)) == Some(std::cmp::Ordering::Greater)
        }
        TypedIndexBounds::GreaterEqual(key) => matches!(
            value_compare::compare_non_null(value, &lit(key)),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
        TypedIndexBounds::LessThan(key) => {
            value_compare::compare_non_null(value, &lit(key)) == Some(std::cmp::Ordering::Less)
        }
        TypedIndexBounds::LessEqual(key) => matches!(
            value_compare::compare_non_null(value, &lit(key)),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        TypedIndexBounds::Range {
            lo,
            lo_inclusive,
            hi,
            hi_inclusive,
        } => {
            let Some(lo_order) = value_compare::compare_non_null(value, &lit(lo)) else {
                return false;
            };
            let Some(hi_order) = value_compare::compare_non_null(value, &lit(hi)) else {
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
