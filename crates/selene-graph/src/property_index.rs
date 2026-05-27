//! Commit-path maintenance for built-in node property indexes.
//!
//! Open-graph data can violate a registered index kind. Mutation-time
//! maintenance logs and skips the single offending `(label, property, value)`
//! update so commits still publish. Registration-time index construction is
//! stricter and returns a typed error because silently creating a partial index
//! would be harder to reason about.

use rustc_hash::FxHashMap;
use selene_core::{IStr, LabelSet, PropertyMap, Value};
use std::collections::BTreeSet;

use crate::error::{GraphError, GraphResult};
use crate::graph::PropertyIndexEntry;
use crate::typed_index::{TypedIndex, TypedIndexKind, TypedIndexValueError};

type PropertyIndexMap = FxHashMap<(IStr, IStr), PropertyIndexEntry>;

pub(crate) fn apply_node_create(
    indexes: &mut PropertyIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
) -> GraphResult<()> {
    for label in labels.iter().copied() {
        for (property, value) in props.iter() {
            if is_null(value) {
                continue;
            }
            insert_commit(indexes, label, *property, value, row)?;
        }
    }
    Ok(())
}

pub(crate) fn apply_node_delete(
    indexes: &mut PropertyIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
) -> GraphResult<()> {
    for label in labels.iter().copied() {
        for (property, value) in props.iter() {
            if is_null(value) {
                continue;
            }
            remove_commit(indexes, label, *property, value, row)?;
        }
    }
    Ok(())
}

pub(crate) fn apply_node_update(
    indexes: &mut PropertyIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
    row: u32,
) -> GraphResult<()> {
    // Iterate only registered indexes whose `(label, property)` is reachable
    // from this update — i.e., the label appears on either side and the
    // property exists on either side. Index-count grows independently of
    // a single node's column footprint, so this keeps update cost
    // proportional to the changed footprint, not the registry size.
    let candidates = candidate_keys(indexes, old_labels, old_props, new_labels, new_props);
    for (label, property) in candidates {
        let old_value = indexable_value(old_labels, old_props, &label, &property);
        let new_value = indexable_value(new_labels, new_props, &label, &property);
        if values_share_key(indexes, label, property, old_value, new_value) {
            continue;
        }
        if let Some(value) = old_value {
            remove_commit(indexes, label, property, value, row)?;
        }
        if let Some(value) = new_value {
            insert_commit(indexes, label, property, value, row)?;
        }
    }
    Ok(())
}

/// Return the set of registered `(label, property)` keys that this update
/// could possibly touch: the label must appear on at least one side of
/// the diff AND the property must appear on at least one side.
fn candidate_keys(
    indexes: &PropertyIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
) -> BTreeSet<(IStr, IStr)> {
    let mut labels: BTreeSet<IStr> = BTreeSet::new();
    labels.extend(old_labels.iter().copied());
    labels.extend(new_labels.iter().copied());

    let mut properties: BTreeSet<IStr> = BTreeSet::new();
    properties.extend(old_props.keys().copied());
    properties.extend(new_props.keys().copied());

    let mut candidates = BTreeSet::new();
    for label in &labels {
        for property in &properties {
            let key = (*label, *property);
            if indexes.contains_key(&key) {
                candidates.insert(key);
            }
        }
    }
    candidates
}

/// Build a property index strictly: any kind mismatch / NaN aborts with a
/// typed [`GraphError::IndexValueRejected`]. Used at registration time
/// (`SharedGraph::create_property_index`) so users learn immediately when
/// existing data violates the declared kind.
pub(crate) fn build_property_index(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: TypedIndexKind,
) -> GraphResult<TypedIndex> {
    build_property_index_inner(graph, label, property, kind, BuildPolicy::Strict)
}

/// Build a property index leniently: kind-mismatched / NaN values are
/// logged and skipped, the same policy used by the commit-path
/// (`insert_commit`). Used by [`rebuild_property_indexes`] so a snapshot
/// that was accepted at runtime (with documented kind drift in open
/// graphs) does not fail recovery — drift is recoverable, fictional
/// indexes are not.
pub(crate) fn build_property_index_lenient(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: TypedIndexKind,
) -> GraphResult<TypedIndex> {
    build_property_index_inner(graph, label, property, kind, BuildPolicy::Lenient)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildPolicy {
    Strict,
    Lenient,
}

fn build_property_index_inner(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: TypedIndexKind,
    policy: BuildPolicy,
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
        match index.insert(value, row) {
            Ok(()) => {}
            Err(err) => match policy {
                BuildPolicy::Strict => return Err(index_rejection(label, property, err)),
                BuildPolicy::Lenient => warn_rejected("rebuild", label, property, row, &err),
            },
        }
    }
    Ok(index)
}

/// Rebuild every registered property index from the graph's columns,
/// using lenient (log-and-skip) semantics on kind mismatches so recovery
/// of a runtime-accepted snapshot never fails. The strict policy lives
/// at registration only.
pub(crate) fn rebuild_property_indexes(graph: &mut crate::SeleneGraph) -> GraphResult<()> {
    let registrations: Vec<((IStr, IStr), TypedIndexKind, Option<IStr>)> = graph
        .property_index
        .iter()
        .map(|(key, entry)| (*key, entry.kind(), entry.name))
        .collect();
    graph.property_index.clear();
    for ((label, property), kind, name) in registrations {
        let index = build_property_index_lenient(graph, label, property, kind)?;
        graph
            .property_index
            .insert((label, property), PropertyIndexEntry::new(index, name));
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
            .is_some_and(|entry| entry.index.values_share_key(old_value, new_value)),
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
) -> GraphResult<()> {
    if let Some(index) = indexes.get_mut(&(label, property))
        && let Err(err) = std::sync::Arc::make_mut(&mut index.index).insert(value, row)
    {
        warn_rejected("insert", label, property, row, &err);
    }
    Ok(())
}

fn remove_commit(
    indexes: &mut PropertyIndexMap,
    label: IStr,
    property: IStr,
    value: &Value,
    row: u32,
) -> GraphResult<()> {
    if let Some(index) = indexes.get_mut(&(label, property))
        && let Err(err) = std::sync::Arc::make_mut(&mut index.index).remove(value, row)
    {
        warn_rejected("remove", label, property, row, &err);
    }
    Ok(())
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
    err: &TypedIndexValueError,
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
