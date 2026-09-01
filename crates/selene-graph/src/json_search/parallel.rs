//! Threshold-gated Rayon helpers for global exact JSON scans.

use roaring::RoaringBitmap;
use selene_core::{CancellationChecker, DbString, JsonPathSelector, JsonValue, NodeId, Value};

use crate::error::GraphError;
use crate::graph::SeleneGraph;
use crate::parallel_scan::{should_parallelize_scan, try_reduce_bitmap_chunks};
use crate::store::RowIndex;

use super::{
    JSON_SEARCH_PARALLEL_CHUNK_ROWS, JSON_SEARCH_PARALLEL_MIN_ROWS, JsonContainmentHit,
    JsonContainmentTopK, JsonPathContainmentHit, JsonPathHit, JsonPathValueHit, JsonPathValueTopK,
    JsonSearchError,
};

/// Borrowed inputs shared by every global JSON scan chunk.
#[derive(Clone, Copy)]
pub(super) struct JsonScan<'a> {
    graph: &'a SeleneGraph,
    label: &'a DbString,
    property: &'a DbString,
}

impl<'a> JsonScan<'a> {
    /// Build shared scan inputs for a label/property JSON scan.
    pub(super) fn new(graph: &'a SeleneGraph, label: &'a DbString, property: &'a DbString) -> Self {
        Self {
            graph,
            label,
            property,
        }
    }

    fn value_for_row(
        self,
        raw_row: u32,
    ) -> Result<Option<(NodeId, &'a JsonValue)>, JsonSearchError> {
        if !self.graph.node_store.is_alive(raw_row) {
            return Ok(None);
        }
        let row = RowIndex::new(raw_row);
        let node_id = self
            .graph
            .node_id_for_row(row)
            .ok_or_else(|| GraphError::Inconsistent {
                reason: format!(
                    "JSON search row {raw_row} for {} has no node id",
                    self.label.as_str()
                ),
            })?;
        let properties = self
            .graph
            .node_store
            .properties
            .get(raw_row as usize)
            .ok_or_else(|| GraphError::Inconsistent {
                reason: format!(
                    "JSON search row {raw_row} for {} has no property row",
                    self.label.as_str()
                ),
            })?;
        Ok(match properties.get(self.property) {
            Some(Value::Json(value)) => Some((node_id, value)),
            _ => None,
        })
    }
}

/// Return true when a global JSON scan should use Rayon.
pub(super) fn should_parallelize_json_scan(rows: &RoaringBitmap, k: usize) -> bool {
    should_parallelize_scan(rows.len(), k, JSON_SEARCH_PARALLEL_MIN_ROWS)
}

/// Parallel implementation of JSON containment scan.
pub(super) fn contains_nodes(
    scan: JsonScan<'_>,
    candidate: &JsonValue,
    k: usize,
    rows: &RoaringBitmap,
    checker: CancellationChecker<'_>,
) -> Result<Vec<JsonContainmentHit>, JsonSearchError> {
    let top_k = try_reduce_bitmap_chunks(
        rows,
        JSON_SEARCH_PARALLEL_CHUNK_ROWS,
        checker,
        || JsonContainmentTopK::new(k),
        |chunk| contains_chunk(scan, candidate, k, chunk),
        merge_node_top_k,
    )?;
    Ok(top_k.into_hits())
}

/// Parallel implementation of JSON path-existence scan.
pub(super) fn path_exists_nodes(
    scan: JsonScan<'_>,
    path: &[JsonPathSelector],
    k: usize,
    rows: &RoaringBitmap,
    checker: CancellationChecker<'_>,
) -> Result<Vec<JsonPathHit>, JsonSearchError> {
    let top_k = try_reduce_bitmap_chunks(
        rows,
        JSON_SEARCH_PARALLEL_CHUNK_ROWS,
        checker,
        || JsonContainmentTopK::new(k),
        |chunk| path_exists_chunk(scan, path, k, chunk),
        merge_node_top_k,
    )?;
    Ok(top_k.into_path_hits())
}

/// Parallel implementation of JSON path-containment scan.
pub(super) fn path_contains_nodes(
    scan: JsonScan<'_>,
    path: &[JsonPathSelector],
    candidate: &JsonValue,
    k: usize,
    rows: &RoaringBitmap,
    checker: CancellationChecker<'_>,
) -> Result<Vec<JsonPathContainmentHit>, JsonSearchError> {
    let top_k = try_reduce_bitmap_chunks(
        rows,
        JSON_SEARCH_PARALLEL_CHUNK_ROWS,
        checker,
        || JsonContainmentTopK::new(k),
        |chunk| path_contains_chunk(scan, path, candidate, k, chunk),
        merge_node_top_k,
    )?;
    Ok(top_k.into_path_containment_hits())
}

/// Parallel implementation of JSON path-value scan.
pub(super) fn path_value_nodes(
    scan: JsonScan<'_>,
    path: &[JsonPathSelector],
    k: usize,
    rows: &RoaringBitmap,
    checker: CancellationChecker<'_>,
) -> Result<Vec<JsonPathValueHit>, JsonSearchError> {
    let top_k = try_reduce_bitmap_chunks(
        rows,
        JSON_SEARCH_PARALLEL_CHUNK_ROWS,
        checker,
        || JsonPathValueTopK::new(k),
        |chunk| path_value_chunk(scan, path, k, chunk),
        merge_value_top_k,
    )?;
    Ok(top_k.into_hits())
}

fn contains_chunk(
    scan: JsonScan<'_>,
    candidate: &JsonValue,
    k: usize,
    rows: &[u32],
) -> Result<JsonContainmentTopK, JsonSearchError> {
    let mut top_k = JsonContainmentTopK::new(k);
    for &raw_row in rows {
        let Some((node_id, value)) = scan.value_for_row(raw_row)? else {
            continue;
        };
        if value.contains(candidate) {
            top_k.push(node_id);
        }
    }
    Ok(top_k)
}

fn path_exists_chunk(
    scan: JsonScan<'_>,
    path: &[JsonPathSelector],
    k: usize,
    rows: &[u32],
) -> Result<JsonContainmentTopK, JsonSearchError> {
    let mut top_k = JsonContainmentTopK::new(k);
    for &raw_row in rows {
        let Some((node_id, value)) = scan.value_for_row(raw_row)? else {
            continue;
        };
        if value.path_exists(path) {
            top_k.push(node_id);
        }
    }
    Ok(top_k)
}

fn path_contains_chunk(
    scan: JsonScan<'_>,
    path: &[JsonPathSelector],
    candidate: &JsonValue,
    k: usize,
    rows: &[u32],
) -> Result<JsonContainmentTopK, JsonSearchError> {
    let mut top_k = JsonContainmentTopK::new(k);
    for &raw_row in rows {
        let Some((node_id, value)) = scan.value_for_row(raw_row)? else {
            continue;
        };
        if value.path_contains(path, candidate) {
            top_k.push(node_id);
        }
    }
    Ok(top_k)
}

fn path_value_chunk(
    scan: JsonScan<'_>,
    path: &[JsonPathSelector],
    k: usize,
    rows: &[u32],
) -> Result<JsonPathValueTopK, JsonSearchError> {
    let mut top_k = JsonPathValueTopK::new(k);
    for &raw_row in rows {
        let Some((node_id, value)) = scan.value_for_row(raw_row)? else {
            continue;
        };
        let Some(selected) = value.path_value_ref(path) else {
            continue;
        };
        top_k.push(node_id, selected);
    }
    Ok(top_k)
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
        lhs.push_owned(hit.node_id, hit.value);
    }
    Ok(lhs)
}
