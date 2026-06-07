//! Threshold-gated Rayon helpers for global exact JSON scans.

use rayon::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{CancellationChecker, DbString, JsonPathSelector, JsonValue, NodeId, Value};

use crate::error::GraphError;
use crate::graph::SeleneGraph;
use crate::store::RowIndex;

use super::{
    JSON_SEARCH_PARALLEL_CHUNK_ROWS, JSON_SEARCH_PARALLEL_MIN_ROWS, JsonContainmentHit,
    JsonContainmentTopK, JsonPathContainmentHit, JsonPathHit, JsonPathValueHit, JsonPathValueTopK,
    JsonSearchError,
};

/// Return true when a global JSON scan should use Rayon.
pub(super) fn should_parallelize_json_scan(
    rows: &RoaringBitmap,
    k: usize,
    checker: CancellationChecker<'_>,
) -> bool {
    checker.is_disabled()
        && k != 0
        && rows.len() >= JSON_SEARCH_PARALLEL_MIN_ROWS
        && rayon::current_num_threads() > 1
}

/// Parallel implementation of JSON containment scan.
pub(super) fn contains_nodes(
    graph: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    candidate: &JsonValue,
    k: usize,
    rows: &RoaringBitmap,
) -> Result<Vec<JsonContainmentHit>, JsonSearchError> {
    let top_k = rows
        .iter()
        .collect::<Vec<_>>()
        .par_chunks(JSON_SEARCH_PARALLEL_CHUNK_ROWS)
        .map(|chunk| contains_chunk(graph, label, property, candidate, k, chunk))
        .try_reduce(|| JsonContainmentTopK::new(k), merge_node_top_k)?;
    Ok(top_k.into_hits())
}

/// Parallel implementation of JSON path-existence scan.
pub(super) fn path_exists_nodes(
    graph: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    path: &[JsonPathSelector],
    k: usize,
    rows: &RoaringBitmap,
) -> Result<Vec<JsonPathHit>, JsonSearchError> {
    let top_k = rows
        .iter()
        .collect::<Vec<_>>()
        .par_chunks(JSON_SEARCH_PARALLEL_CHUNK_ROWS)
        .map(|chunk| path_exists_chunk(graph, label, property, path, k, chunk))
        .try_reduce(|| JsonContainmentTopK::new(k), merge_node_top_k)?;
    Ok(top_k.into_path_hits())
}

/// Parallel implementation of JSON path-containment scan.
pub(super) fn path_contains_nodes(
    graph: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    path: &[JsonPathSelector],
    candidate: &JsonValue,
    k: usize,
    rows: &RoaringBitmap,
) -> Result<Vec<JsonPathContainmentHit>, JsonSearchError> {
    let top_k = rows
        .iter()
        .collect::<Vec<_>>()
        .par_chunks(JSON_SEARCH_PARALLEL_CHUNK_ROWS)
        .map(|chunk| path_contains_chunk(graph, label, property, path, candidate, k, chunk))
        .try_reduce(|| JsonContainmentTopK::new(k), merge_node_top_k)?;
    Ok(top_k.into_path_containment_hits())
}

/// Parallel implementation of JSON path-value scan.
pub(super) fn path_value_nodes(
    graph: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    path: &[JsonPathSelector],
    k: usize,
    rows: &RoaringBitmap,
) -> Result<Vec<JsonPathValueHit>, JsonSearchError> {
    let top_k = rows
        .iter()
        .collect::<Vec<_>>()
        .par_chunks(JSON_SEARCH_PARALLEL_CHUNK_ROWS)
        .map(|chunk| path_value_chunk(graph, label, property, path, k, chunk))
        .try_reduce(|| JsonPathValueTopK::new(k), merge_value_top_k)?;
    Ok(top_k.into_hits())
}

fn contains_chunk(
    graph: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    candidate: &JsonValue,
    k: usize,
    rows: &[u32],
) -> Result<JsonContainmentTopK, JsonSearchError> {
    let mut top_k = JsonContainmentTopK::new(k);
    for &raw_row in rows {
        let Some((node_id, value)) = json_value_for_row(graph, label, property, raw_row)? else {
            continue;
        };
        if value.contains(candidate) {
            top_k.push(node_id);
        }
    }
    Ok(top_k)
}

fn path_exists_chunk(
    graph: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    path: &[JsonPathSelector],
    k: usize,
    rows: &[u32],
) -> Result<JsonContainmentTopK, JsonSearchError> {
    let mut top_k = JsonContainmentTopK::new(k);
    for &raw_row in rows {
        let Some((node_id, value)) = json_value_for_row(graph, label, property, raw_row)? else {
            continue;
        };
        if value.path_exists(path) {
            top_k.push(node_id);
        }
    }
    Ok(top_k)
}

fn path_contains_chunk(
    graph: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    path: &[JsonPathSelector],
    candidate: &JsonValue,
    k: usize,
    rows: &[u32],
) -> Result<JsonContainmentTopK, JsonSearchError> {
    let mut top_k = JsonContainmentTopK::new(k);
    for &raw_row in rows {
        let Some((node_id, value)) = json_value_for_row(graph, label, property, raw_row)? else {
            continue;
        };
        if value.path_contains(path, candidate) {
            top_k.push(node_id);
        }
    }
    Ok(top_k)
}

fn path_value_chunk(
    graph: &SeleneGraph,
    label: &DbString,
    property: &DbString,
    path: &[JsonPathSelector],
    k: usize,
    rows: &[u32],
) -> Result<JsonPathValueTopK, JsonSearchError> {
    let mut top_k = JsonPathValueTopK::new(k);
    for &raw_row in rows {
        let Some((node_id, value)) = json_value_for_row(graph, label, property, raw_row)? else {
            continue;
        };
        let Some(selected) = value.path_value(path) else {
            continue;
        };
        top_k.push(node_id, selected);
    }
    Ok(top_k)
}

fn json_value_for_row<'a>(
    graph: &'a SeleneGraph,
    label: &DbString,
    property: &DbString,
    raw_row: u32,
) -> Result<Option<(NodeId, &'a JsonValue)>, JsonSearchError> {
    if !graph.node_store.is_alive(raw_row) {
        return Ok(None);
    }
    let row = RowIndex::new(raw_row);
    let node_id = graph
        .node_id_for_row(row)
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!(
                "JSON search row {raw_row} for {} has no node id",
                label.as_str()
            ),
        })?;
    let properties = graph
        .node_store
        .properties
        .get(raw_row as usize)
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!(
                "JSON search row {raw_row} for {} has no property row",
                label.as_str()
            ),
        })?;
    Ok(match properties.get(property) {
        Some(Value::Json(value)) => Some((node_id, value)),
        _ => None,
    })
}

fn merge_node_top_k(
    mut lhs: JsonContainmentTopK,
    rhs: JsonContainmentTopK,
) -> Result<JsonContainmentTopK, JsonSearchError> {
    for hit in rhs.into_hits() {
        lhs.push(hit.node_id);
    }
    Ok(lhs)
}

fn merge_value_top_k(
    mut lhs: JsonPathValueTopK,
    rhs: JsonPathValueTopK,
) -> Result<JsonPathValueTopK, JsonSearchError> {
    for hit in rhs.into_hits() {
        lhs.push(hit.node_id, hit.value);
    }
    Ok(lhs)
}
