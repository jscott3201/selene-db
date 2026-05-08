//! Commit-path maintenance for built-in node property indexes.
//!
//! Open-graph data can violate a registered index kind. Mutation-time
//! maintenance logs and skips the single offending `(label, property, value)`
//! update so commits still publish. Registration-time index construction is
//! stricter and returns a typed error because silently creating a partial index
//! would be harder to reason about.

use std::sync::Arc;

use imbl::HashMap;
use selene_core::{IStr, LabelSet, PropertyMap, Value};

use crate::error::{GraphError, GraphResult};
use crate::typed_index::{TypedIndex, TypedIndexKind, TypedIndexValueError};

type PropertyIndexMap = HashMap<(IStr, IStr), Arc<TypedIndex>>;

pub(crate) fn apply_node_create(
    indexes: &mut PropertyIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
) {
    for label in labels.iter().copied() {
        for (property, value) in props.iter() {
            if is_null(value) {
                continue;
            }
            insert_commit(indexes, label, *property, value, row);
        }
    }
}

pub(crate) fn apply_node_delete(
    indexes: &mut PropertyIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
) {
    for label in labels.iter().copied() {
        for (property, value) in props.iter() {
            if is_null(value) {
                continue;
            }
            remove_commit(indexes, label, *property, value, row);
        }
    }
}

pub(crate) fn apply_node_update(
    indexes: &mut PropertyIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
    row: u32,
) {
    let registered: Vec<(IStr, IStr)> = indexes.keys().copied().collect();
    for (label, property) in registered {
        let old_value = indexable_value(old_labels, old_props, &label, &property);
        let new_value = indexable_value(new_labels, new_props, &label, &property);
        if values_share_key(indexes, label, property, old_value, new_value) {
            continue;
        }
        if let Some(value) = old_value {
            remove_commit(indexes, label, property, value, row);
        }
        if let Some(value) = new_value {
            insert_commit(indexes, label, property, value, row);
        }
    }
}

pub(crate) fn build_property_index(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: TypedIndexKind,
) -> GraphResult<TypedIndex> {
    let mut index = TypedIndex::new(kind);
    for row_index in 0..graph.node_store.labels.len() {
        let row = u32::try_from(row_index).map_err(|_| GraphError::Inconsistent {
            reason: format!(
                "node store row index {row_index} exceeds u32::MAX; selene-graph \
                 caps rows at u32::MAX",
            ),
        })?;
        if !graph.node_store.is_alive(row) {
            continue;
        }
        let Some(labels) = graph.node_store.labels.get(row_index) else {
            continue;
        };
        if !labels.contains(&label) {
            continue;
        }
        let Some(props) = graph.node_store.properties.get(row_index) else {
            continue;
        };
        let Some(value) = props.get(&property) else {
            continue;
        };
        if is_null(value) {
            continue;
        }
        index
            .insert(value, row)
            .map_err(|err| index_rejection(label, property, err))?;
    }
    Ok(index)
}

pub(crate) fn rebuild_property_indexes(graph: &mut crate::SeleneGraph) -> GraphResult<()> {
    let registrations: Vec<((IStr, IStr), TypedIndexKind)> = graph
        .property_index
        .iter()
        .map(|(key, index)| (*key, index.kind()))
        .collect();
    graph.property_index.clear();
    for ((label, property), kind) in registrations {
        let index = build_property_index(graph, label, property, kind)?;
        graph
            .property_index
            .insert((label, property), Arc::new(index));
    }
    Ok(())
}

fn values_share_key(
    indexes: &PropertyIndexMap,
    label: IStr,
    property: IStr,
    old_value: Option<&Value>,
    new_value: Option<&Value>,
) -> bool {
    match (old_value, new_value) {
        (None, None) => true,
        (Some(old_value), Some(new_value)) => indexes
            .get(&(label, property))
            .is_some_and(|index| index.values_share_key(old_value, new_value)),
        _ => false,
    }
}

fn indexable_value<'a>(
    labels: &LabelSet,
    props: &'a PropertyMap,
    label: &IStr,
    property: &IStr,
) -> Option<&'a Value> {
    if !labels.contains(label) {
        return None;
    }
    props.get(property).filter(|value| !is_null(value))
}

fn insert_commit(
    indexes: &mut PropertyIndexMap,
    label: IStr,
    property: IStr,
    value: &Value,
    row: u32,
) {
    if let Some(index) = indexes.get_mut(&(label, property))
        && let Err(err) = Arc::make_mut(index).insert(value, row)
    {
        warn_rejected("insert", label, property, row, err);
    }
}

fn remove_commit(
    indexes: &mut PropertyIndexMap,
    label: IStr,
    property: IStr,
    value: &Value,
    row: u32,
) {
    if let Some(index) = indexes.get_mut(&(label, property))
        && let Err(err) = Arc::make_mut(index).remove(value, row)
    {
        warn_rejected("remove", label, property, row, err);
    }
}

fn index_rejection(label: IStr, property: IStr, err: TypedIndexValueError) -> GraphError {
    GraphError::IndexValueRejected {
        label,
        property,
        expected_kind: err.expected_kind(),
        observed: err.observed(),
    }
}

fn warn_rejected(
    op: &'static str,
    label: IStr,
    property: IStr,
    row: u32,
    err: TypedIndexValueError,
) {
    tracing::warn!(
        op,
        %label,
        %property,
        row,
        expected_kind = ?err.expected_kind(),
        observed = err.observed(),
        "skipped property-index update for value that does not match the registered index kind",
    );
}

const fn is_null(value: &Value) -> bool {
    matches!(value, Value::Null)
}

#[cfg(test)]
#[path = "property_index_tests.rs"]
mod tests;
