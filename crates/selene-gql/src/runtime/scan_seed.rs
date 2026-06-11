//! Seed-bound scan short-circuit for correlated pattern execution.

use selene_core::Value;

use crate::{
    NodeOrEdgeScan, PatternPlan, ScanAccess, ScanKind,
    runtime::{Binding, BindingTableSchema, EvalCtx, ExecutorError},
};

use super::{
    scan, scan_bind,
    scan_resolve::{range_satisfiable_runtime, resolve_bitmap_union_key_values, resolve_bounds},
};

pub(super) fn try_seeded_scan(
    scan: &NodeOrEdgeScan,
    pattern: &PatternPlan,
    schema: &BindingTableSchema,
    seed: &Binding,
    slots: scan_bind::ScanSlots,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Option<Vec<(Value, Binding)>>, ExecutorError> {
    let Some(index) = slots.binding_index() else {
        return Ok(None);
    };
    let Some(seed_value) = seed.get(index) else {
        return Ok(None);
    };
    let Some((entity, row)) = seeded_entity_row(scan.kind, seed_value, ctx) else {
        return Ok(None);
    };
    let Some(row) = row else {
        return Ok(Some(Vec::new()));
    };
    if !scan::label_matches_scan(scan, row, ctx) || !value_constraint_passes(scan, row, ctx)? {
        return Ok(Some(Vec::new()));
    }
    let Some(binding) = scan_bind::binding_for_scan(schema, Some(seed), entity.clone(), slots)
    else {
        return Ok(Some(Vec::new()));
    };
    if !scan::predicates_pass(scan, pattern, &binding, schema, &entity, ctx)? {
        return Ok(Some(Vec::new()));
    }
    Ok(Some(vec![(entity, binding)]))
}

fn seeded_entity_row(
    kind: ScanKind,
    seed_value: &Value,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Option<(Value, Option<u32>)> {
    let snapshot = ctx.tx.snapshot();
    match (kind, seed_value) {
        (ScanKind::Node, Value::NodeRef(id)) => {
            let row = snapshot
                .is_node_alive(*id)
                .then(|| snapshot.row_for_node_id(*id))
                .flatten()
                .map(|row| row.get());
            Some((Value::NodeRef(*id), row))
        }
        (ScanKind::Edge, Value::EdgeRef(id)) => {
            let row = snapshot
                .is_edge_alive(*id)
                .then(|| snapshot.row_for_edge_id(*id))
                .flatten()
                .map(|row| row.get());
            Some((Value::EdgeRef(*id), row))
        }
        _ => None,
    }
}

fn value_constraint_passes(
    scan: &NodeOrEdgeScan,
    row: u32,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<bool, ExecutorError> {
    match &scan.access {
        ScanAccess::Linear | ScanAccess::LabelIndex { .. } => Ok(true),
        ScanAccess::TypedIndexRange {
            property,
            kind,
            bounds,
            ..
        } => {
            let Some(resolved) = resolve_bounds(bounds, *kind, ctx)? else {
                return Ok(false);
            };
            Ok(range_satisfiable_runtime(&resolved)
                && scan::row_matches_resolved_bounds(scan.kind, row, property, &resolved, ctx))
        }
        ScanAccess::BitmapUnion {
            property,
            kind,
            keys,
            ..
        } => {
            let mut resolved = Vec::with_capacity(keys.len());
            for key in keys {
                resolved.extend(resolve_bitmap_union_key_values(key, *kind, ctx)?);
            }
            Ok((!resolved.is_empty() || keys.is_empty())
                && scan::property_matches_any_resolved(scan.kind, row, property, &resolved, ctx))
        }
        ScanAccess::CompositeLookup {
            properties, keys, ..
        } => {
            let Some(values) = scan::resolve_composite_values(properties, keys, ctx)? else {
                return Ok(false);
            };
            Ok(scan::row_matches_resolved_composite(
                scan.kind, row, properties, &values, ctx,
            ))
        }
    }
}
