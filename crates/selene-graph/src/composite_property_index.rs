//! Commit-path maintenance for built-in composite node property indexes.

use rustc_hash::FxHashMap;
use selene_core::{IStr, LabelSet, PropertyMap, Value};
use smallvec::SmallVec;

use crate::composite_typed_index::CompositeIndexValueError;
use crate::error::{GraphError, GraphResult};
use crate::graph::{CompositePropertyIndexEntry, composite_property_key};
use crate::{CompositeTypedIndex, TypedIndexKind};

type CompositeIndexMap = FxHashMap<(IStr, SmallVec<[IStr; 4]>), CompositePropertyIndexEntry>;

pub(crate) fn apply_node_create(
    indexes: &mut CompositeIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
) -> GraphResult<()> {
    for (label, entry) in indexes_for_labels(indexes, labels) {
        if let Some(values) = indexable_values(props, &entry.declared_properties) {
            insert_commit(label, entry, &values, row)?;
        }
    }
    Ok(())
}

pub(crate) fn apply_node_delete(
    indexes: &mut CompositeIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
) -> GraphResult<()> {
    for (label, entry) in indexes_for_labels(indexes, labels) {
        if let Some(values) = indexable_values(props, &entry.declared_properties) {
            remove_commit(label, entry, &values, row)?;
        }
    }
    Ok(())
}

pub(crate) fn apply_node_update(
    indexes: &mut CompositeIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
    row: u32,
) -> GraphResult<()> {
    for ((label, _), entry) in indexes.iter_mut() {
        if !old_labels.contains(label) && !new_labels.contains(label) {
            continue;
        }
        let old_values = old_labels
            .contains(label)
            .then(|| indexable_values(old_props, &entry.declared_properties))
            .flatten();
        let new_values = new_labels
            .contains(label)
            .then(|| indexable_values(new_props, &entry.declared_properties))
            .flatten();
        if values_share_key(entry, old_values.as_ref(), new_values.as_ref()) {
            continue;
        }
        if let Some(values) = old_values {
            remove_commit(*label, entry, &values, row)?;
        }
        if let Some(values) = new_values {
            insert_commit(*label, entry, &values, row)?;
        }
    }
    Ok(())
}

/// Build a composite property index strictly.
pub(crate) fn build_composite_property_index(
    graph: &crate::SeleneGraph,
    label: IStr,
    properties: SmallVec<[IStr; 4]>,
    kinds: SmallVec<[TypedIndexKind; 4]>,
) -> GraphResult<CompositeTypedIndex> {
    build_composite_property_index_inner(graph, label, properties, kinds, BuildPolicy::Strict)
}

/// Build a composite property index leniently.
pub(crate) fn build_composite_property_index_lenient(
    graph: &crate::SeleneGraph,
    label: IStr,
    properties: SmallVec<[IStr; 4]>,
    kinds: SmallVec<[TypedIndexKind; 4]>,
) -> GraphResult<CompositeTypedIndex> {
    build_composite_property_index_inner(graph, label, properties, kinds, BuildPolicy::Lenient)
}

/// Rebuild every registered composite property index from graph columns.
pub(crate) fn rebuild_composite_property_indexes(
    graph: &mut crate::SeleneGraph,
) -> GraphResult<()> {
    let registrations: Vec<_> = graph
        .composite_property_index
        .iter()
        .map(|((label, _), entry)| {
            (
                *label,
                entry.declared_properties.clone(),
                entry.kinds(),
                entry.name,
            )
        })
        .collect();
    graph.composite_property_index.clear();
    for (label, properties, kinds, name) in registrations {
        let key = composite_property_key(&properties);
        let index =
            build_composite_property_index_lenient(graph, label, properties.clone(), kinds)?;
        graph.composite_property_index.insert(
            (label, key),
            CompositePropertyIndexEntry::new(index, properties, name),
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildPolicy {
    Strict,
    Lenient,
}

fn build_composite_property_index_inner(
    graph: &crate::SeleneGraph,
    label: IStr,
    properties: SmallVec<[IStr; 4]>,
    kinds: SmallVec<[TypedIndexKind; 4]>,
    policy: BuildPolicy,
) -> GraphResult<CompositeTypedIndex> {
    let mut index = CompositeTypedIndex::new(kinds);
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
        let Some(values) = indexable_values(props, &properties) else {
            continue;
        };
        match index.insert(&values, row) {
            Ok(()) => {}
            Err(err) => match policy {
                BuildPolicy::Strict => return Err(index_rejection(label, &properties, err)),
                BuildPolicy::Lenient => warn_rejected("rebuild", label, &properties, row, &err),
            },
        }
    }
    Ok(index)
}

fn indexes_for_labels<'a>(
    indexes: &'a mut CompositeIndexMap,
    labels: &'a LabelSet,
) -> impl Iterator<Item = (IStr, &'a mut CompositePropertyIndexEntry)> {
    indexes
        .iter_mut()
        .filter_map(|((label, _), entry)| labels.contains(label).then_some((*label, entry)))
}

fn indexable_values<'a>(
    props: &'a PropertyMap,
    properties: &[IStr],
) -> Option<SmallVec<[&'a Value; 4]>> {
    properties
        .iter()
        .map(|property| props.get(property).filter(|value| !is_null(value)))
        .collect()
}

fn values_share_key(
    entry: &CompositePropertyIndexEntry,
    old_values: Option<&SmallVec<[&Value; 4]>>,
    new_values: Option<&SmallVec<[&Value; 4]>>,
) -> bool {
    match (old_values, new_values) {
        (None, None) => true,
        (Some(old_values), Some(new_values)) => {
            entry.index.values_share_key(old_values, new_values)
        }
        _ => false,
    }
}

fn insert_commit(
    label: IStr,
    entry: &mut CompositePropertyIndexEntry,
    values: &[&Value],
    row: u32,
) -> GraphResult<()> {
    if let Err(err) = std::sync::Arc::make_mut(&mut entry.index).insert(values, row) {
        warn_rejected("insert", label, &entry.declared_properties, row, &err);
    }
    Ok(())
}

fn remove_commit(
    label: IStr,
    entry: &mut CompositePropertyIndexEntry,
    values: &[&Value],
    row: u32,
) -> GraphResult<()> {
    if let Err(err) = std::sync::Arc::make_mut(&mut entry.index).remove(values, row) {
        warn_rejected("remove", label, &entry.declared_properties, row, &err);
    }
    Ok(())
}

fn index_rejection(label: IStr, properties: &[IStr], err: CompositeIndexValueError) -> GraphError {
    match err {
        CompositeIndexValueError::ArityMismatch { expected, observed } => {
            GraphError::Inconsistent {
                reason: format!(
                    "composite index ({label}, {properties:?}) expected {expected} values but observed {observed}"
                ),
            }
        }
        CompositeIndexValueError::Component {
            index,
            expected_kind,
            observed,
        } => GraphError::IndexValueRejected {
            label,
            property: properties
                .get(index)
                .copied()
                .unwrap_or_else(|| properties.first().copied().unwrap_or(label)),
            expected_kind,
            observed,
        },
        // Commit 1 (BRIEF-153) preserves the existing silent-skip lenient
        // behavior; Commit 2 routes this through a dedicated
        // `GraphError::IndexAdmissionExhausted` so cap-exhaustion details
        // surface intact. Carrying the IStr-pool source string in the
        // observed slot until the variant lands keeps the lenient path
        // diagnostic-equivalent for the moment.
        CompositeIndexValueError::ComponentAdmissionFailed {
            index,
            expected_kind,
            reason: _,
        } => GraphError::IndexValueRejected {
            label,
            property: properties
                .get(index)
                .copied()
                .unwrap_or_else(|| properties.first().copied().unwrap_or(label)),
            expected_kind,
            observed: "ExternalString",
        },
    }
}

fn warn_rejected(
    op: &'static str,
    label: IStr,
    properties: &[IStr],
    row: u32,
    err: &CompositeIndexValueError,
) {
    tracing::warn!(
        op,
        %label,
        ?properties,
        row,
        ?err,
        "skipped composite-property-index update for value tuple that does not match the registered index kinds",
    );
}

const fn is_null(value: &Value) -> bool {
    matches!(value, Value::Null)
}

#[cfg(test)]
#[path = "composite_property_index_tests.rs"]
mod tests;
