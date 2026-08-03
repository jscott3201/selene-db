//! Commit-path maintenance for built-in composite node property indexes.

use rustc_hash::FxHashMap;
use selene_core::{DbString, LabelSet, PropertyMap, Value};
use smallvec::SmallVec;

use crate::composite_typed_index::CompositeIndexValueError;
use crate::error::{GraphError, GraphResult};
use crate::graph::{CompositePropertyIndexEntry, composite_property_key};
use crate::{CompositeTypedIndex, TypedIndexKind};

type CompositeIndexMap =
    FxHashMap<(DbString, SmallVec<[DbString; 4]>), CompositePropertyIndexEntry>;

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
            remove_commit(label.clone(), entry, &values, row)?;
        }
        if let Some(values) = new_values {
            insert_commit(label.clone(), entry, &values, row)?;
        }
    }
    Ok(())
}

/// Build a composite property index strictly.
pub(crate) fn build_composite_property_index(
    graph: &crate::SeleneGraph,
    label: DbString,
    properties: SmallVec<[DbString; 4]>,
    kinds: SmallVec<[TypedIndexKind; 4]>,
) -> GraphResult<CompositeTypedIndex> {
    build_composite_property_index_inner(graph, label, properties, kinds, BuildPolicy::Strict)
        .map(|built| built.index)
}

/// Build a composite property index leniently.
pub(crate) fn build_composite_property_index_lenient(
    graph: &crate::SeleneGraph,
    label: DbString,
    properties: SmallVec<[DbString; 4]>,
    kinds: SmallVec<[TypedIndexKind; 4]>,
) -> GraphResult<LenientCompositeBuild> {
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
                label.clone(),
                entry.declared_properties.clone(),
                entry.kinds(),
                entry.name.clone(),
            )
        })
        .collect();
    graph.composite_property_index.clear();
    for (label, properties, kinds, name) in registrations {
        let key = composite_property_key(&properties);
        let built = build_composite_property_index_lenient(
            graph,
            label.clone(),
            properties.clone(),
            kinds,
        )?;
        graph.composite_property_index.insert(
            (label, key),
            CompositePropertyIndexEntry::new_with_drift(
                built.index,
                properties,
                name,
                built.drifted_rows,
            ),
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildPolicy {
    Strict,
    Lenient,
}

/// A built composite index plus the number of live rows it could not key.
pub(crate) struct LenientCompositeBuild {
    /// The index over every row whose components all matched their kinds.
    pub(crate) index: CompositeTypedIndex,
    /// Rows skipped because the index could not key them.
    pub(crate) drifted_rows: u64,
}

/// Whether a skipped tuple leaves the index an incomplete view of its columns.
///
/// A composite tuple is all-or-nothing, so one unusable component drops the
/// whole row. NaN is excluded for the same reason as the single-key path: it
/// satisfies no equality or range predicate, so a scan omits the row too.
///
/// [`CompositeIndexValueError`] has no NaN variant; a genuine NaN arrives as
/// `Component` with `observed == "NaN"` and must be recognised by that string,
/// not by the discriminant. `ArityMismatch` counts, because a row the index
/// cannot key is a row the index is missing.
fn counts_as_drift(err: &CompositeIndexValueError) -> bool {
    match err {
        CompositeIndexValueError::Component { observed, .. } => *observed != "NaN",
        CompositeIndexValueError::ArityMismatch { .. } => true,
    }
}

fn build_composite_property_index_inner(
    graph: &crate::SeleneGraph,
    label: DbString,
    properties: SmallVec<[DbString; 4]>,
    kinds: SmallVec<[TypedIndexKind; 4]>,
    policy: BuildPolicy,
) -> GraphResult<LenientCompositeBuild> {
    let mut index = CompositeTypedIndex::new(kinds);
    let mut drifted_rows = 0_u64;
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
                BuildPolicy::Strict => {
                    return Err(index_rejection(label.clone(), &properties, err));
                }
                BuildPolicy::Lenient => {
                    if counts_as_drift(&err) {
                        drifted_rows = drifted_rows.saturating_add(1);
                    }
                    warn_rejected("rebuild", label.clone(), &properties, row, &err);
                }
            },
        }
    }
    Ok(LenientCompositeBuild {
        index,
        drifted_rows,
    })
}

fn indexes_for_labels<'a>(
    indexes: &'a mut CompositeIndexMap,
    labels: &'a LabelSet,
) -> impl Iterator<Item = (DbString, &'a mut CompositePropertyIndexEntry)> {
    indexes
        .iter_mut()
        .filter_map(|((label, _), entry)| labels.contains(label).then_some((label.clone(), entry)))
}

fn indexable_values<'a>(
    props: &'a PropertyMap,
    properties: &[DbString],
) -> Option<SmallVec<[&'a Value; 4]>> {
    properties
        .iter()
        .map(|property| props.get(property).filter(|value| !is_null(value)))
        .collect()
}

/// Whether an update can skip index maintenance entirely.
///
/// Deliberately does not reuse [`CompositeTypedIndex::values_share_key`], which
/// treats any two unkeyable tuples as sharing a key. That is true of the
/// bitmaps — neither tuple is in the index either way — but it is not true of
/// the drift tally. A row moving between a NaN tuple and a kind-mismatched one
/// changes whether it counts, and skipping would strand the count: the index
/// would answer while still missing the row.
fn values_share_key(
    entry: &CompositePropertyIndexEntry,
    old_values: Option<&SmallVec<[&Value; 4]>>,
    new_values: Option<&SmallVec<[&Value; 4]>>,
) -> bool {
    match (old_values, new_values) {
        (None, None) => true,
        (Some(old_values), Some(new_values)) => {
            match (
                entry.index.key_from_values(old_values),
                entry.index.key_from_values(new_values),
            ) {
                (Ok(old_key), Ok(new_key)) => old_key == new_key,
                (Err(old_err), Err(new_err)) => {
                    counts_as_drift(&old_err) == counts_as_drift(&new_err)
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn insert_commit(
    label: DbString,
    entry: &mut CompositePropertyIndexEntry,
    values: &[&Value],
    row: u32,
) -> GraphResult<()> {
    let Err(err) = std::sync::Arc::make_mut(&mut entry.index).insert(values, row) else {
        return Ok(());
    };
    if counts_as_drift(&err) {
        entry.drifted_rows = entry.drifted_rows.saturating_add(1);
    }
    demote_or_promote(label, &entry.declared_properties, row, "insert", err)
}

fn remove_commit(
    label: DbString,
    entry: &mut CompositePropertyIndexEntry,
    values: &[&Value],
    row: u32,
) -> GraphResult<()> {
    let Err(err) = std::sync::Arc::make_mut(&mut entry.index).remove(values, row) else {
        return Ok(());
    };
    if counts_as_drift(&err) {
        entry.drifted_rows = entry.drifted_rows.saturating_sub(1);
    }
    demote_or_promote(label, &entry.declared_properties, row, "remove", err)
}

/// Commit-path branching for [`CompositeIndexValueError`]: parallel to the
/// single-key helper in [`crate::property_index`]. `Component` (kind
/// mismatch) AND `ArityMismatch` retain the commit-path semantics of
/// `warn_rejected` lenient skip. Build paths handle `ArityMismatch`
/// separately via [`index_rejection`] under the strict policy.
fn demote_or_promote(
    label: DbString,
    properties: &[DbString],
    row: u32,
    op: &'static str,
    err: CompositeIndexValueError,
) -> GraphResult<()> {
    match err {
        CompositeIndexValueError::Component { .. }
        | CompositeIndexValueError::ArityMismatch { .. } => {
            warn_rejected(op, label, properties, row, &err);
            Ok(())
        }
    }
}

fn index_rejection(
    label: DbString,
    properties: &[DbString],
    err: CompositeIndexValueError,
) -> GraphError {
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
            property: properties
                .get(index)
                .cloned()
                .unwrap_or_else(|| properties.first().cloned().unwrap_or_else(|| label.clone())),
            label,
            expected_kind,
            observed,
        },
    }
}

fn warn_rejected(
    op: &'static str,
    label: DbString,
    properties: &[DbString],
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
