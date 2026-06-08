//! Integrity-check bodies for the `selene.verify` native built-in.
//!
//! Ported verbatim from the historical procedure-pack `verify` built-in. Each
//! `check_*` function walks one class of graph invariant and returns a [`CheckResult`]
//! summarizing the inconsistencies it found; the orchestration and row shaping
//! live in the parent [`super`] module.

use selene_core::{DbString, EdgeId, NodeId, Value};
use selene_graph::{
    AdjacencyEntry, CompositeKey, CompositeKeyComponent, CompositeTypedIndex, NotNanF64, RowIndex,
    SeleneGraph, TypedIndex,
};

use super::CheckResult;

pub(super) fn check_label_index_cardinality(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut indexed_rows = 0_u64;
    let mut expected_rows = 0_u64;

    for (label, bitmap) in &snapshot.idx_label {
        indexed_rows += bitmap.len();
        for row in bitmap {
            if !live_node_row(snapshot, row) {
                issues += 1;
                continue;
            }
            let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
                issues += 1;
                continue;
            };
            if !labels.contains(label) {
                issues += 1;
            }
        }
    }

    for row in snapshot.live_nodes() {
        let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
            issues += 1;
            continue;
        };
        for label in labels.iter() {
            expected_rows += 1;
            if !snapshot
                .nodes_with_label(label)
                .is_some_and(|bitmap| bitmap.contains(row))
            {
                issues += 1;
            }
        }
    }

    if indexed_rows != expected_rows {
        issues += indexed_rows.abs_diff(expected_rows) as usize;
    }
    CheckResult::new(
        issues,
        format!(
            "indexed label rows={indexed_rows}; expected label rows={expected_rows}; issues={issues}"
        ),
    )
}

pub(super) fn check_property_index_coverage(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut indexed_rows = 0_u64;
    let mut expected_rows = 0_u64;

    for ((label, property), entry) in &snapshot.property_index {
        indexed_rows += entry.index.cardinality();
        for row in snapshot.live_nodes() {
            let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
                issues += 1;
                continue;
            };
            if !labels.contains(label) {
                continue;
            }
            let Some(properties) = snapshot.node_store.properties.get(row as usize) else {
                issues += 1;
                continue;
            };
            let Some(value) = properties.get(property) else {
                continue;
            };
            if let Some(bitmap) = snapshot.nodes_with_property_eq(label, property, value) {
                expected_rows += 1;
                if !bitmap.contains(row) {
                    issues += 1;
                }
            }
        }
    }
    for ((label, _), entry) in &snapshot.composite_property_index {
        indexed_rows += entry.index.cardinality();
        for row in snapshot.live_nodes() {
            let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
                issues += 1;
                continue;
            };
            if !labels.contains(label) {
                continue;
            }
            let Some(properties) = snapshot.node_store.properties.get(row as usize) else {
                issues += 1;
                continue;
            };
            let Some(values) = composite_property_values(properties, &entry.declared_properties)
            else {
                continue;
            };
            // With a single string space `key_from_values` resolves every
            // coercible STRING component directly; an `Err` (arity / kind
            // mismatch) means this row is not indexable, so skip it.
            let key = match entry.index.key_from_values(&values) {
                Ok(key) => key,
                Err(_) => continue,
            };
            expected_rows += 1;
            if !entry
                .index
                .lookup_key(&key)
                .is_some_and(|bitmap| bitmap.contains(row))
            {
                issues += 1;
            }
        }
    }

    if indexed_rows != expected_rows {
        issues += indexed_rows.abs_diff(expected_rows) as usize;
    }
    CheckResult::new(
        issues,
        format!(
            "indexed property rows={indexed_rows}; expected property rows={expected_rows}; issues={issues}"
        ),
    )
}

pub(super) fn check_adjacency_symmetry(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut outgoing_edges = 0_usize;
    let mut incoming_edges = 0_usize;
    let mut live_edges = 0_usize;

    for (source, entry) in &snapshot.adjacency_out {
        for edge in entry.iter() {
            outgoing_edges += 1;
            if !snapshot.is_node_alive(*source) || !snapshot.is_node_alive(edge.neighbor) {
                issues += 1;
            }
            match expected_edge(snapshot, edge.edge_id) {
                Some((actual_source, actual_target, actual_label)) => {
                    if actual_source != *source || actual_target != edge.neighbor {
                        issues += 1;
                    }
                    if actual_label != edge.label {
                        issues += 1;
                    }
                }
                None => {
                    issues += 1;
                }
            }
            if !snapshot
                .incoming_edges(edge.neighbor)
                .is_some_and(|incoming| {
                    incoming.iter().any(|candidate| {
                        candidate.edge_id == edge.edge_id
                            && candidate.neighbor == *source
                            && candidate.label == edge.label
                    })
                })
            {
                issues += 1;
            }
        }
    }

    for (target, entry) in &snapshot.adjacency_in {
        for edge in entry.iter() {
            incoming_edges += 1;
            if !snapshot.is_node_alive(*target) || !snapshot.is_node_alive(edge.neighbor) {
                issues += 1;
            }
            match expected_edge(snapshot, edge.edge_id) {
                Some((actual_source, actual_target, actual_label)) => {
                    if actual_source != edge.neighbor || actual_target != *target {
                        issues += 1;
                    }
                    if actual_label != edge.label {
                        issues += 1;
                    }
                }
                None => {
                    issues += 1;
                }
            }
            if !snapshot
                .outgoing_edges(edge.neighbor)
                .is_some_and(|outgoing| {
                    outgoing.iter().any(|candidate| {
                        candidate.edge_id == edge.edge_id
                            && candidate.neighbor == *target
                            && candidate.label == edge.label
                    })
                })
            {
                issues += 1;
            }
        }
    }

    for row in &snapshot.edge_store.alive {
        live_edges += 1;
        let Some(edge_id) = snapshot.edge_id_for_row(RowIndex::new(row)) else {
            issues += 1;
            continue;
        };
        let Some((source, target, label)) = expected_edge(snapshot, edge_id) else {
            issues += 1;
            continue;
        };
        if !adjacency_entry_contains(
            snapshot.outgoing_edges(source),
            target,
            edge_id,
            label.clone(),
        ) {
            issues += 1;
        }
        if !adjacency_entry_contains(snapshot.incoming_edges(target), source, edge_id, label) {
            issues += 1;
        }
    }

    if outgoing_edges != live_edges {
        issues += outgoing_edges.abs_diff(live_edges);
    }
    if incoming_edges != live_edges {
        issues += incoming_edges.abs_diff(live_edges);
    }
    CheckResult::new(
        issues,
        format!(
            "live edges={live_edges}; outgoing adjacency edges={outgoing_edges}; incoming adjacency edges={incoming_edges}; issues={issues}"
        ),
    )
}

fn expected_edge(snapshot: &SeleneGraph, edge_id: EdgeId) -> Option<(NodeId, NodeId, DbString)> {
    let (source, target) = snapshot.edge_endpoints(edge_id)?;
    let label = snapshot.edge_label(edge_id)?.clone();
    Some((source, target, label))
}

fn adjacency_entry_contains(
    entry: Option<&AdjacencyEntry>,
    neighbor: NodeId,
    edge_id: EdgeId,
    label: DbString,
) -> bool {
    entry.is_some_and(|entry| {
        entry
            .iter()
            .any(|edge| edge.neighbor == neighbor && edge.edge_id == edge_id && edge.label == label)
    })
}

pub(super) fn check_edge_endpoint_liveness(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut checked = 0_usize;

    for row in &snapshot.edge_store.alive {
        checked += 1;
        let Some(edge_id) = snapshot.edge_id_for_row(RowIndex::new(row)) else {
            issues += 1;
            continue;
        };
        match snapshot.edge_endpoints(edge_id) {
            Some((source, target))
                if snapshot.is_node_alive(source) && snapshot.is_node_alive(target) => {}
            _ => {
                issues += 1;
            }
        }
    }

    CheckResult::new(
        issues,
        format!("live edges checked={checked}; issues={issues}"),
    )
}

pub(super) fn check_typed_index_value_range(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut checked = 0_u64;

    for ((label, property), entry) in &snapshot.property_index {
        for (bucket, row) in typed_index_entries(&entry.index) {
            checked += 1;
            if !indexed_property_row_matches(snapshot, label.clone(), property.clone(), row, bucket)
            {
                issues += 1;
            }
        }
    }
    for ((label, _), entry) in &snapshot.composite_property_index {
        for (bucket, row) in composite_index_entries(&entry.index) {
            checked += 1;
            if !indexed_composite_row_matches(
                snapshot,
                label.clone(),
                &entry.declared_properties,
                row,
                &bucket,
            ) {
                issues += 1;
            }
        }
    }

    CheckResult::new(
        issues,
        format!("typed index rows checked={checked}; issues={issues}"),
    )
}

pub(super) fn check_roaring_bitmap_density(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut bitmaps = 0_usize;

    for bitmap in snapshot.idx_label.values() {
        bitmaps += 1;
        for row in bitmap {
            if !live_node_row(snapshot, row) {
                issues += 1;
            }
        }
    }
    for bitmap in snapshot.idx_edge_label.values() {
        bitmaps += 1;
        for row in bitmap {
            if !live_edge_row(snapshot, row) {
                issues += 1;
            }
        }
    }
    for entry in snapshot.property_index.values() {
        for (_bucket, row) in typed_index_entries(&entry.index) {
            if !live_node_row(snapshot, row) {
                issues += 1;
            }
        }
    }
    for entry in snapshot.composite_property_index.values() {
        for (_bucket, row) in composite_index_entries(&entry.index) {
            if !live_node_row(snapshot, row) {
                issues += 1;
            }
        }
    }

    CheckResult::new(
        issues,
        format!("bitmap groups checked={bitmaps}; issues={issues}"),
    )
}

fn typed_index_entries(index: &TypedIndex) -> Vec<(IndexedValue, u32)> {
    let mut entries = Vec::new();
    match index {
        TypedIndex::Bool(index) => {
            for (key, bitmap) in index {
                push_index_entries(&mut entries, IndexedValue::Bool(*key), bitmap.iter());
            }
        }
        TypedIndex::I64(index) => {
            for (key, bitmap) in index {
                push_index_entries(&mut entries, IndexedValue::I64(*key), bitmap.iter());
            }
        }
        TypedIndex::F64(index) => {
            for (key, bitmap) in index {
                push_index_entries(&mut entries, IndexedValue::F64(*key), bitmap.iter());
            }
        }
        TypedIndex::String(index) => {
            for (key, bitmap) in index {
                push_index_entries(
                    &mut entries,
                    IndexedValue::String(key.clone()),
                    bitmap.iter(),
                );
            }
        }
        TypedIndex::Date(index) => {
            for (key, bitmap) in index {
                push_index_entries(&mut entries, IndexedValue::Date(*key), bitmap.iter());
            }
        }
        TypedIndex::LocalDateTime(index) => {
            for (key, bitmap) in index {
                push_index_entries(
                    &mut entries,
                    IndexedValue::LocalDateTime(*key),
                    bitmap.iter(),
                );
            }
        }
        TypedIndex::Uuid(index) => {
            for (key, bitmap) in index {
                push_index_entries(
                    &mut entries,
                    IndexedValue::Uuid(key.to_string()),
                    bitmap.iter(),
                );
            }
        }
    }
    entries
}

fn push_index_entries(
    entries: &mut Vec<(IndexedValue, u32)>,
    key: IndexedValue,
    rows: impl Iterator<Item = u32>,
) {
    entries.extend(rows.map(|row| (key.clone(), row)));
}

fn composite_index_entries(index: &CompositeTypedIndex) -> Vec<(CompositeKey, u32)> {
    let mut entries = Vec::new();
    for (key, bitmap) in index.entries() {
        entries.extend(bitmap.iter().map(|row| (key.clone(), row)));
    }
    entries
}

fn indexed_property_row_matches(
    snapshot: &SeleneGraph,
    label: DbString,
    property: DbString,
    row: u32,
    bucket: IndexedValue,
) -> bool {
    if !live_node_row(snapshot, row) {
        return false;
    }
    let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
        return false;
    };
    if !labels.contains(&label) {
        return false;
    }
    let Some(properties) = snapshot.node_store.properties.get(row as usize) else {
        return false;
    };
    let Some(value) = properties.get(&property) else {
        return false;
    };
    bucket_matches_value(bucket, value)
}

fn indexed_composite_row_matches(
    snapshot: &SeleneGraph,
    label: DbString,
    properties: &[DbString],
    row: u32,
    bucket: &CompositeKey,
) -> bool {
    if !live_node_row(snapshot, row) || bucket.len() != properties.len() {
        return false;
    }
    let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
        return false;
    };
    if !labels.contains(&label) {
        return false;
    }
    let Some(row_properties) = snapshot.node_store.properties.get(row as usize) else {
        return false;
    };
    properties.iter().zip(bucket).all(|(property, component)| {
        row_properties
            .get(property)
            .is_some_and(|value| component_matches_value(component, value))
    })
}

fn composite_property_values<'a>(
    row_properties: &'a selene_core::PropertyMap,
    properties: &[DbString],
) -> Option<Vec<&'a Value>> {
    properties
        .iter()
        .map(|property| row_properties.get(property))
        .collect()
}

#[derive(Clone, Debug)]
enum IndexedValue {
    Bool(bool),
    I64(i64),
    F64(NotNanF64),
    String(DbString),
    Date(jiff::civil::Date),
    LocalDateTime(jiff::civil::DateTime),
    Uuid(String),
}

fn bucket_matches_value(bucket: IndexedValue, value: &Value) -> bool {
    match (bucket, value) {
        (IndexedValue::Bool(expected), Value::Bool(actual)) => expected == *actual,
        (IndexedValue::I64(expected), Value::Int(actual)) => expected == *actual,
        (IndexedValue::F64(expected), Value::Float(actual)) => {
            NotNanF64::new(*actual).is_ok_and(|actual| actual == expected)
        }
        (IndexedValue::String(expected), Value::String(actual)) => expected == *actual,
        (IndexedValue::Date(expected), Value::Date(actual)) => expected == *actual,
        (IndexedValue::LocalDateTime(expected), Value::LocalDateTime(actual)) => {
            expected == *actual
        }
        (IndexedValue::Uuid(expected), Value::Uuid(actual)) => expected == actual.to_string(),
        _ => false,
    }
}

fn component_matches_value(component: &CompositeKeyComponent, value: &Value) -> bool {
    match (component, value) {
        (CompositeKeyComponent::Bool(expected), Value::Bool(actual)) => expected == actual,
        (CompositeKeyComponent::I64(expected), Value::Int(actual)) => expected == actual,
        (CompositeKeyComponent::F64(expected), Value::Float(actual)) => {
            NotNanF64::new(*actual).is_ok_and(|actual| actual == *expected)
        }
        (CompositeKeyComponent::String(expected), Value::String(actual)) => expected == actual,
        (CompositeKeyComponent::Date(expected), Value::Date(actual)) => expected == actual,
        (CompositeKeyComponent::LocalDateTime(expected), Value::LocalDateTime(actual)) => {
            expected == actual
        }
        (CompositeKeyComponent::Uuid(expected), Value::Uuid(actual)) => expected == actual,
        _ => false,
    }
}

fn live_node_row(snapshot: &SeleneGraph, row: u32) -> bool {
    (row as usize) < snapshot.node_store.len() && snapshot.node_store.is_alive(row)
}

fn live_edge_row(snapshot: &SeleneGraph, row: u32) -> bool {
    (row as usize) < snapshot.edge_store.len() && snapshot.edge_store.is_alive(row)
}
