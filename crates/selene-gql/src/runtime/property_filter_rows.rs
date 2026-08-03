//! Index-or-scan row filters for procedure property filters.
//!
//! Procedure filters (`selene.text_search_nodes`, `selene.vector_search_nodes_ann`,
//! `algo.pagerank`) admit candidates through an indexed property. They require
//! the property to be indexed and report a bad argument when it is not.
//!
//! A registered index still declines to answer when it omits live rows whose
//! variant it cannot key, because an incomplete index would silently drop
//! candidates. These helpers scan in that case, using the same cross-variant
//! equality the executor's own fallback uses, so a filtered procedure returns
//! what a complete index would have returned instead of failing or under-
//! reporting.

use roaring::RoaringBitmap;
use selene_core::{DbString, Value};
use selene_graph::SeleneGraph;

use super::value_compare;

/// Node rows of `label` whose `property` equals any of `values`.
///
/// `None` means the caller supplied a property with no registered index, or
/// values the registered index cannot key — the argument errors the procedures
/// already report. It never means "the index is incomplete"; that case scans.
pub(crate) fn node_rows_with_property_any(
    snapshot: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    values: &[Value],
) -> Option<RoaringBitmap> {
    let entry = snapshot
        .property_index
        .get(&(label.clone(), property.clone()))?;
    if let Some(rows) = snapshot.nodes_with_property_any(label, property, values) {
        return Some(rows);
    }
    if entry.is_complete() {
        return None;
    }
    let mut rows = RoaringBitmap::new();
    for row in snapshot.live_nodes() {
        let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
            continue;
        };
        if !labels.contains(label) {
            continue;
        }
        let Some(properties) = snapshot.node_store.properties.get(row as usize) else {
            continue;
        };
        if matches_any(properties.get(property), values) {
            rows.insert(row);
        }
    }
    Some(rows)
}

/// Edge rows of `label` whose `property` equals any of `values`.
///
/// Mirrors [`node_rows_with_property_any`] over the edge store.
pub(crate) fn edge_rows_with_property_any(
    snapshot: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    values: &[Value],
) -> Option<RoaringBitmap> {
    let entry = snapshot
        .edge_property_index
        .get(&(label.clone(), property.clone()))?;
    if let Some(rows) = snapshot.edges_with_property_any(label, property, values) {
        return Some(rows);
    }
    if entry.is_complete() {
        return None;
    }
    let mut rows = RoaringBitmap::new();
    for row in snapshot.live_edges() {
        let Some(edge_label) = snapshot.edge_store.label.get(row as usize) else {
            continue;
        };
        if edge_label != label {
            continue;
        }
        let Some(properties) = snapshot.edge_store.properties.get(row as usize) else {
            continue;
        };
        if matches_any(properties.get(property), values) {
            rows.insert(row);
        }
    }
    Some(rows)
}

/// Whether a stored property value equals any supplied filter value.
///
/// NULL never matches on either side, matching the index path, where NULLs are
/// excluded before a key is ever built.
fn matches_any(stored: Option<&Value>, values: &[Value]) -> bool {
    let Some(stored) = stored else {
        return false;
    };
    if matches!(stored, Value::Null) {
        return false;
    }
    values
        .iter()
        .filter(|candidate| !matches!(candidate, Value::Null))
        .any(|candidate| value_compare::equal_non_null(stored, candidate))
}
