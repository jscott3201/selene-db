//! Single-scan pattern executor.

use std::collections::BTreeSet;
use std::ops::Bound::{Excluded, Included, Unbounded};

use selene_core::{IStr, LabelSet, Value};
use selene_graph::RowIndex;

use crate::{
    FilterPredicate, FilterPredicateKind, IndexKey, IndexKind, LabelExpr, NodeOrEdgeScan,
    PatternPlan, ScanAccess, ScanKind, TypedIndexBounds,
    runtime::{Binding, BindingTableSchema, ExecutorError},
};

use super::scan_resolve::{
    IndexKeyOutcome, ResolvedBounds, range_satisfiable_runtime, resolve_bounds, resolve_index_key,
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
    for row in candidate_rows(scan, ctx)? {
        if !label_matches_scan(scan, row, ctx) {
            continue;
        }
        let Some(entity) = entity_value(scan.kind, row, ctx) else {
            continue;
        };
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

fn candidate_rows(
    scan: &NodeOrEdgeScan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<u32>, ExecutorError> {
    match &scan.access {
        ScanAccess::Linear => Ok(linear_rows(scan.kind, ctx)),
        ScanAccess::LabelIndex { .. } => Ok(label_index_rows(scan, ctx)),
        ScanAccess::TypedIndexRange {
            property,
            kind,
            bounds,
            ..
        } => typed_index_rows(scan, property.clone(), *kind, bounds, ctx),
        ScanAccess::BitmapUnion {
            property,
            kind,
            keys,
            ..
        } => bitmap_union_rows(scan, property.clone(), *kind, keys, ctx),
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
    kind: IndexKind,
    bounds: &TypedIndexBounds,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<u32>, ExecutorError> {
    // Pre-resolve every IndexKey in the bounds against the bound parameters
    // up front (BRIEF-154 §B.3). `None` means at least one slot resolved to
    // `IndexKeyOutcome::EmptyResult` — short-circuit the whole probe to an
    // empty result without erroring.
    let Some(resolved) = resolve_bounds(bounds, kind, ctx)? else {
        return Ok(Vec::new());
    };
    // Plan-time `range_satisfiable` only gates literal-literal pairs; for
    // parameter-bearing ranges the resolved values may be unsatisfiable
    // (`$lo > $hi`, or `$lo == $hi` with both exclusive). Forwarding those
    // to `nodes_with_property_range` → `BTreeMap::range` would std::panic.
    // Mirror the plan-time guard against the resolved values and bail to
    // an empty result on unsatisfiable (Codex PR #175 F1).
    if !range_satisfiable_runtime(&resolved) {
        return Ok(Vec::new());
    }
    if scan.kind != ScanKind::Node {
        return Ok(linear_rows_filtered_by_resolved_bounds(
            scan, property, &resolved, ctx,
        ));
    }
    let Some(label) = single_label(&scan.label_predicate) else {
        return Ok(linear_rows_filtered_by_resolved_bounds(
            scan, property, &resolved, ctx,
        ));
    };
    let indexed_rows = match &resolved {
        ResolvedBounds::Equality(value) => ctx
            .tx
            .snapshot()
            .nodes_with_property_eq(&label, &property, value)
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        ResolvedBounds::GreaterThan(value) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Excluded(value.clone()), Unbounded))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        ResolvedBounds::GreaterEqual(value) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Included(value.clone()), Unbounded))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        ResolvedBounds::LessThan(value) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Unbounded, Excluded(value.clone())))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        ResolvedBounds::LessEqual(value) => ctx
            .tx
            .snapshot()
            .nodes_with_property_range(&label, &property, (Unbounded, Included(value.clone())))
            .map(|rows| rows.iter().collect::<Vec<_>>()),
        ResolvedBounds::Range {
            lo,
            lo_inclusive,
            hi,
            hi_inclusive,
        } => {
            let lo_bound = if *lo_inclusive {
                Included(lo.clone())
            } else {
                Excluded(lo.clone())
            };
            let hi_bound = if *hi_inclusive {
                Included(hi.clone())
            } else {
                Excluded(hi.clone())
            };
            ctx.tx
                .snapshot()
                .nodes_with_property_range(&label, &property, (lo_bound, hi_bound))
                .map(|rows| rows.iter().collect::<Vec<_>>())
        }
    };
    Ok(indexed_rows
        .unwrap_or_else(|| linear_rows_filtered_by_resolved_bounds(scan, property, &resolved, ctx)))
}

fn bitmap_union_rows(
    scan: &NodeOrEdgeScan,
    property: IStr,
    kind: IndexKind,
    keys: &[IndexKey],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<u32>, ExecutorError> {
    Ok(union_property_eq(scan, property, kind, keys, ctx)?
        .into_iter()
        .collect())
}

fn union_property_eq(
    scan: &NodeOrEdgeScan,
    property: IStr,
    kind: IndexKind,
    keys: &[IndexKey],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BTreeSet<u32>, ExecutorError> {
    // Pre-resolve all keys once: parameter slots resolve to concrete Values
    // for the variant-strict equality probes the BitmapUnion fans out.
    // `IndexKeyOutcome::EmptyResult` slots (NULL binding) drop out of the
    // union per BRIEF-154 §B.3. A loud `InvalidParameterType` propagates
    // immediately.
    let mut resolved_keys: Vec<Value> = Vec::with_capacity(keys.len());
    for key in keys {
        match resolve_index_key(key, kind, ctx)? {
            IndexKeyOutcome::Value(value) => resolved_keys.push(value),
            IndexKeyOutcome::EmptyResult => {}
        }
    }
    // P3 short-circuit: if every key resolved to EmptyResult, the union is
    // empty by construction — no need to scan rows looking for matches.
    if resolved_keys.is_empty() && !keys.is_empty() {
        return Ok(BTreeSet::new());
    }
    if scan.kind != ScanKind::Node {
        return Ok(linear_rows(scan.kind, ctx)
            .into_iter()
            .filter(|row| {
                property_matches_any_resolved(
                    scan.kind,
                    *row,
                    property.clone(),
                    &resolved_keys,
                    ctx,
                )
            })
            .collect());
    }
    let Some(label) = single_label(&scan.label_predicate) else {
        return Ok(linear_rows(scan.kind, ctx)
            .into_iter()
            .filter(|row| {
                property_matches_any_resolved(
                    scan.kind,
                    *row,
                    property.clone(),
                    &resolved_keys,
                    ctx,
                )
            })
            .collect());
    };
    let mut rows = BTreeSet::new();
    let mut used_index = false;
    for value in &resolved_keys {
        if let Some(matches) = ctx
            .tx
            .snapshot()
            .nodes_with_property_eq(&label, &property, value)
        {
            used_index = true;
            rows.extend(matches.iter());
        }
    }
    if used_index {
        Ok(rows)
    } else {
        Ok(linear_rows(scan.kind, ctx)
            .into_iter()
            .filter(|row| {
                property_matches_any_resolved(
                    scan.kind,
                    *row,
                    property.clone(),
                    &resolved_keys,
                    ctx,
                )
            })
            .collect())
    }
}

fn composite_lookup_rows(
    scan: &NodeOrEdgeScan,
    properties: &[(IStr, IndexKind)],
    keys: &[(IStr, IndexKey)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Vec<u32>, ExecutorError> {
    // Resolve once at scan-entry: every key is matched to its declared kind
    // (per BRIEF-154 §B.2 F7/F17). Any EmptyResult short-circuits the whole
    // probe to an empty result; errors propagate.
    let Some(resolved_values) = resolve_composite_values(properties, keys, ctx)? else {
        return Ok(Vec::new());
    };
    if scan.kind != ScanKind::Node {
        return Ok(linear_rows_filtered_by_resolved_composite(
            scan,
            properties,
            &resolved_values,
            ctx,
        ));
    }
    let Some(label) = single_label(&scan.label_predicate) else {
        return Ok(linear_rows_filtered_by_resolved_composite(
            scan,
            properties,
            &resolved_values,
            ctx,
        ));
    };
    let property_keys: Vec<IStr> = properties
        .iter()
        .map(|(property, _)| property.clone())
        .collect();
    if let Some(index) = ctx
        .tx
        .snapshot()
        .composite_property_index_for(&label, &property_keys)
    {
        let refs = resolved_values.iter().collect::<Vec<_>>();
        // Single-coercion key build; a kind/arity mismatch (`Err`) declines the
        // index probe and drops to the linear filter below.
        if let Ok(key) = index.key_from_values(&refs) {
            return Ok(index
                .lookup_key(&key)
                .map(|bitmap| bitmap.iter().collect())
                .unwrap_or_default());
        }
    }
    Ok(linear_rows_filtered_by_resolved_composite(
        scan,
        properties,
        &resolved_values,
        ctx,
    ))
}

/// Resolve a composite probe's per-component keys against bound parameters.
///
/// Returns `Ok(None)` when any component resolved to `EmptyResult` (e.g. a
/// NULL parameter binding). Returns `Ok(Some(values))` with `values` aligned
/// to `properties` order.
fn resolve_composite_values(
    properties: &[(IStr, IndexKind)],
    keys: &[(IStr, IndexKey)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Option<Vec<Value>>, ExecutorError> {
    let mut out = Vec::with_capacity(properties.len());
    for (property, kind) in properties {
        let Some((_, key)) = keys.iter().find(|(name, _)| name == property) else {
            // Optimizer guarantees property/key alignment; if it diverges we
            // can't probe the composite at all.
            return Ok(None);
        };
        // Composite probes via `composite_property_index_for` →
        // `key_from_values` are variant-strict on each component.
        match resolve_index_key(key, *kind, ctx)? {
            IndexKeyOutcome::Value(value) => out.push(value),
            IndexKeyOutcome::EmptyResult => return Ok(None),
        }
    }
    Ok(Some(out))
}

fn linear_rows_filtered_by_resolved_composite(
    scan: &NodeOrEdgeScan,
    properties: &[(IStr, IndexKind)],
    values: &[Value],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<u32> {
    linear_rows(scan.kind, ctx)
        .into_iter()
        .filter(|row| {
            properties
                .iter()
                .zip(values.iter())
                .all(|((property, _), expected)| {
                    property_value(scan.kind, *row, property.clone(), ctx)
                        .is_some_and(|value| value_eq_non_null(value, expected))
                })
        })
        .collect()
}

fn linear_rows_filtered_by_resolved_bounds(
    scan: &NodeOrEdgeScan,
    property: IStr,
    resolved: &ResolvedBounds,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Vec<u32> {
    linear_rows(scan.kind, ctx)
        .into_iter()
        .filter(|row| {
            property_value(scan.kind, *row, property.clone(), ctx)
                .is_some_and(|value| value_matches_resolved_bounds(value, resolved))
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
        .position(|column| column.name == Some(binding_def.name.clone()))?;
    binding.get(index).cloned()
}

fn label_matches_scan(scan: &NodeOrEdgeScan, row: u32, ctx: &EvalCtx<'_, '_, '_, '_>) -> bool {
    let Some(label_expr) = &scan.label_predicate else {
        return true;
    };
    let snapshot = ctx.tx.snapshot();
    match scan.kind {
        ScanKind::Node => {
            let Some(id) = snapshot.node_id_for_row(RowIndex::new(row)) else {
                return false;
            };
            snapshot
                .node_labels(id)
                .is_some_and(|labels| label_matches_node(label_expr, labels))
        }
        ScanKind::Edge => {
            let Some(id) = snapshot.edge_id_for_row(RowIndex::new(row)) else {
                return false;
            };
            snapshot
                .edge_label(id)
                .is_some_and(|label| label_matches_edge(label_expr, label.clone()))
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
        LabelExpr::Conjunction(parts) => parts
            .iter()
            .all(|part| label_matches_edge(part, label.clone())),
        LabelExpr::Disjunction(parts) => parts
            .iter()
            .any(|part| label_matches_edge(part, label.clone())),
        LabelExpr::Negation(part) => !label_matches_edge(part, label),
        LabelExpr::Wildcard => true,
    }
}

fn single_label(label: &Option<LabelExpr>) -> Option<IStr> {
    match label {
        Some(LabelExpr::Single(label)) => Some(label.clone()),
        _ => None,
    }
}

fn entity_value(kind: ScanKind, row: u32, ctx: &EvalCtx<'_, '_, '_, '_>) -> Option<Value> {
    let snapshot = ctx.tx.snapshot();
    match kind {
        ScanKind::Node => snapshot
            .node_id_for_row(RowIndex::new(row))
            .map(Value::NodeRef),
        ScanKind::Edge => snapshot
            .edge_id_for_row(RowIndex::new(row))
            .map(Value::EdgeRef),
    }
}

fn property_matches_any_resolved(
    kind: ScanKind,
    row: u32,
    property: IStr,
    values: &[Value],
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> bool {
    property_value(kind, row, property, ctx).is_some_and(|value| {
        values
            .iter()
            .any(|expected| value_eq_non_null(value, expected))
    })
}

fn property_value<'a>(
    kind: ScanKind,
    row: u32,
    property: IStr,
    ctx: &'a EvalCtx<'_, '_, '_, '_>,
) -> Option<&'a Value> {
    let snapshot = ctx.tx.snapshot();
    match kind {
        ScanKind::Node => snapshot
            .node_id_for_row(RowIndex::new(row))
            .and_then(|id| snapshot.node_properties(id))
            .and_then(|properties| properties.get(&property)),
        ScanKind::Edge => snapshot
            .edge_id_for_row(RowIndex::new(row))
            .and_then(|id| snapshot.edge_properties(id))
            .and_then(|properties| properties.get(&property)),
    }
}

fn value_matches_resolved_bounds(value: &Value, resolved: &ResolvedBounds) -> bool {
    match resolved {
        ResolvedBounds::Equality(expected) => value_eq_non_null(value, expected),
        ResolvedBounds::GreaterThan(expected) => {
            value_compare::compare_non_null(value, expected) == Some(std::cmp::Ordering::Greater)
        }
        ResolvedBounds::GreaterEqual(expected) => matches!(
            value_compare::compare_non_null(value, expected),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
        ResolvedBounds::LessThan(expected) => {
            value_compare::compare_non_null(value, expected) == Some(std::cmp::Ordering::Less)
        }
        ResolvedBounds::LessEqual(expected) => matches!(
            value_compare::compare_non_null(value, expected),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        ResolvedBounds::Range {
            lo,
            lo_inclusive,
            hi,
            hi_inclusive,
        } => {
            let Some(lo_order) = value_compare::compare_non_null(value, lo) else {
                return false;
            };
            let Some(hi_order) = value_compare::compare_non_null(value, hi) else {
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

fn value_eq_non_null(lhs: &Value, rhs: &Value) -> bool {
    if matches!(lhs, Value::Null) || matches!(rhs, Value::Null) {
        return false;
    }
    value_compare::equal_non_null(lhs, rhs)
}
