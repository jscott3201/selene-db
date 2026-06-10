//! Derived-state rebuild and recovery-validation helpers for
//! [`SharedGraph`](super::SharedGraph) construction: label/adjacency/index
//! rebuilds, id-map reconstruction, and unique-provider tag validation.
//! Split from the parent file to keep both under the repository 700-LOC cap.
//! `rebuild_derived_state` and `validate_unique_provider_tags` keep their
//! `crate::shared::` paths through the parent's pub(crate) re-export.

use super::*;

pub(crate) fn rebuild_derived_state(graph: &mut SeleneGraph) -> GraphResult<()> {
    graph.idx_label.clear();
    graph.idx_edge_label.clear();
    graph.adjacency_out.clear();
    graph.adjacency_in.clear();

    let node_count = graph.node_store.labels.len();
    for row_index in 0..node_count {
        let row = u32::try_from(row_index).map_err(|_| GraphError::Inconsistent {
            reason: format!(
                "node store row index {row_index} exceeds u32::MAX; selene-graph \
                 caps rows at u32::MAX",
            ),
        })?;
        if !graph.node_store.is_alive(row) {
            continue;
        }
        if let Some(labels) = graph.node_store.labels.get(row_index) {
            for label in labels.iter() {
                graph
                    .idx_label
                    .entry(label.clone())
                    .or_default()
                    .insert(row);
            }
        }
    }

    let edge_count = graph.edge_store.label.len();
    for row_index in 0..edge_count {
        let row = u32::try_from(row_index).map_err(|_| GraphError::Inconsistent {
            reason: format!(
                "edge store row index {row_index} exceeds u32::MAX; selene-graph \
                 caps rows at u32::MAX",
            ),
        })?;
        if !graph.edge_store.is_alive(row) {
            continue;
        }
        if let Some(label) = graph.edge_store.label.get(row_index) {
            graph
                .idx_edge_label
                .entry(label.clone())
                .or_default()
                .insert(row);
        }
    }
    // BRIEF-Item-4a: bind external ids to rows BEFORE rebuild_adjacency, which
    // (from Increment 3) reads the edge id from the row_to_id column.
    rebuild_id_maps(graph)?;
    rebuild_adjacency(graph)?;
    Ok(())
}

/// Rebuild the external-id <-> [`RowIndex`] maps from the per-store `row_to_id`
/// columns, as the final id-binding step of a full rebuild (recovery /
/// [`SharedGraph`] construction).
///
/// This is the id-map authority for the recovery path — the live commit path
/// populates the maps directly in the mutator and never reaches here. The
/// `row_to_id` column is the persisted source of truth: recovery's
/// `insert_*_row` writes the real external id for every materialized row (alive
/// **and** dead — the snapshot persists dead rows so a deleted id stays
/// resolvable to its dead row -> `NotAlive`, matching the live path) and the
/// tombstone for never-committed aborted-tx holes. Seeding the maps from the
/// column therefore preserves the live/recovery id-map equality exactly; only
/// holes are skipped (-> `None` -> `NotFound`, the accepted refinement).
///
/// For an externally-built graph that never populated `row_to_id` (e.g. a test
/// that pushes store columns directly), an alive row whose column slot is still
/// the tombstone falls back to the `row + 1` identity binding. That fallback is
/// the only `row + 1` arithmetic here and is allowlisted in the BRIEF-Item-4a
/// STEP-8 grep-gate; BRIEF-Item-4b drops it once every construction path
/// persists ids.
fn rebuild_id_maps(graph: &mut SeleneGraph) -> GraphResult<()> {
    graph.node_id_to_row.clear();
    graph.edge_id_to_row.clear();
    // Externally-built graphs may not have populated row_to_id; pad it to the
    // row-column length with tombstones so every materialized row is in-bounds.
    let node_len = graph.node_store.len();
    let edge_len = graph.edge_store.len();
    pad_row_to_id(
        &mut graph.node_store.row_to_id,
        node_len,
        selene_core::NodeId::TOMBSTONE,
    );
    pad_row_to_id(
        &mut graph.edge_store.row_to_id,
        edge_len,
        selene_core::EdgeId::TOMBSTONE,
    );

    for row in 0..node_len {
        let raw = row as u32;
        let mut id = graph
            .node_store
            .row_to_id
            .get(row)
            .copied()
            .unwrap_or(selene_core::NodeId::TOMBSTONE);
        if id == selene_core::NodeId::TOMBSTONE {
            // Hole (never committed) -> stays out of the map. An alive row with
            // no persisted id is an externally-built graph; fall back to the
            // identity binding and repair the column.
            if !graph.node_store.is_alive(raw) {
                continue;
            }
            id = selene_core::NodeId::new(u64::from(raw) + 1); // rowid-arith-ok: 4a identity bootstrap (externally-built graph); 4b reads the persisted id
            graph.node_store.row_to_id.set(row, id);
        }
        graph.node_id_to_row.insert(id, RowIndex::new(raw));
    }
    for row in 0..edge_len {
        let raw = row as u32;
        let mut id = graph
            .edge_store
            .row_to_id
            .get(row)
            .copied()
            .unwrap_or(selene_core::EdgeId::TOMBSTONE);
        if id == selene_core::EdgeId::TOMBSTONE {
            if !graph.edge_store.is_alive(raw) {
                continue;
            }
            id = selene_core::EdgeId::new(u64::from(raw) + 1); // rowid-arith-ok: 4a identity bootstrap (externally-built graph); 4b reads the persisted id
            graph.edge_store.row_to_id.set(row, id);
        }
        graph.edge_id_to_row.insert(id, RowIndex::new(raw));
    }
    Ok(())
}

/// Grow `column` with `tombstone` until it is at least `target_len` long.
///
/// [`crate::chunked_vec::ChunkedVec`] supports only push/set, so this pads but
/// never shrinks. In practice the recovery insert path already keeps the column
/// at row-column length (this is then a no-op), while an externally-built graph
/// is padded from empty.
fn pad_row_to_id<T: Clone>(
    column: &mut crate::chunked_vec::ChunkedVec<T>,
    target_len: usize,
    tombstone: T,
) {
    while column.len() < target_len {
        column.push(tombstone.clone());
    }
}

fn rebuild_adjacency(graph: &mut SeleneGraph) -> GraphResult<()> {
    let edge_count = graph.edge_store.len();
    for row_index in 0..edge_count {
        let row = u32::try_from(row_index).map_err(|_| GraphError::Inconsistent {
            reason: format!(
                "edge store row index {row_index} exceeds u32::MAX; selene-graph \
                 caps rows at u32::MAX",
            ),
        })?;
        if !graph.edge_store.is_alive(row) {
            continue;
        }
        let (label, source, target) = edge_row_parts(&graph.edge_store, row_index)?;
        // rebuild_id_maps ran first, so the edge id is read from the row_to_id
        // column (the persistence-stable id), never synthesized as row + 1.
        let edge_id =
            graph
                .edge_id_for_row(RowIndex::new(row))
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "alive edge row {row} has no mapped external id during rebuild"
                    ),
                })?;
        graph
            .adjacency_out
            .entry(source)
            .or_default()
            .add(AdjacencyEdge {
                label: label.clone(),
                neighbor: target,
                edge_id,
            });
        graph
            .adjacency_in
            .entry(target)
            .or_default()
            .add(AdjacencyEdge {
                label,
                neighbor: source,
                edge_id,
            });
    }
    Ok(())
}

fn edge_row_parts(
    store: &EdgeStore,
    row_index: usize,
) -> GraphResult<(DbString, selene_core::NodeId, selene_core::NodeId)> {
    let label = store
        .label
        .get(row_index)
        .cloned()
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!("edge label column missing row {row_index}"),
        })?;
    let source = *store
        .source
        .get(row_index)
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!("edge source column missing row {row_index}"),
        })?;
    let target = *store
        .target
        .get(row_index)
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!("edge target column missing row {row_index}"),
        })?;
    Ok((label, source, target))
}

pub(crate) fn validate_unique_provider_tags(
    providers: &[Arc<dyn IndexProvider>],
) -> GraphResult<()> {
    let mut seen = std::collections::BTreeSet::new();
    for provider in providers {
        let tag = provider.provider_tag();
        if !seen.insert(tag) {
            return Err(GraphError::Provider(ProviderError::Inconsistent {
                reason: format!("duplicate provider tag {tag}"),
            }));
        }
    }
    Ok(())
}
