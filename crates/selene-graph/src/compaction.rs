//! CORE graph compaction mechanism (D23, BRIEF-Item-4b).
//!
//! [`compact_core`] is a pure transform: given a [`SeleneGraph`], it builds a
//! fresh graph whose rows are dense (every dead / aborted-tx hole row dropped),
//! preserving external `NodeId` / `EdgeId` and the monotonic allocator
//! high-water marks, then rebuilds all derived state from the compacted columns
//! via the existing recovery-path rebuilders. It performs NO publication and NO
//! snapshot I/O — the snapshot writer wiring is BRIEF-Item-4c, the
//! create-time row-allocation change (arith → append) and dropping the
//! `rebuild_id_maps` identity bootstrap land when a compacted graph first goes
//! live (also 4c), and the global IStr pool is untouched (its reclamation is a
//! fresh-process reload property, not in-process — see the audit doc, Item 12).
//!
//! Because 4a left edges + adjacency keyed by stable external `NodeId`, a row
//! renumber does not touch edge endpoints or adjacency — only the row-keyed
//! columns, the alive bitmaps, the row-indexed label/property indexes, and the
//! id↔row maps move, and all of those are rebuilt from the compacted columns.
//!
//! **4e constraint (load-bearing — do not lose this thread).** Compaction drops
//! dead rows, so a *deleted* external id that gets compacted away resolves
//! `NotFound` afterwards (was `NotAlive` under 4a's Option B). That flip makes
//! the naive WAL-replay path FAIL, not no-op: recovery
//! ([`crate::core_provider`] `recovery_state`) routes `Change::NodeDeleted` /
//! `EdgeDeleted` (and any post-delete update) through `require_live_*`, which
//! *hard-errors* when the id is absent from the decoded snapshot. A WAL entry
//! written *before* a compaction that touches a since-reclaimed id, replayed
//! *after* loading the compacted snapshot, would abort recovery. BRIEF-Item-4e
//! MUST resolve this — preferably by recording a compaction WAL floor in the
//! MANIFEST (D15) so pre-compaction WAL entries are never replayed against a
//! compacted snapshot; alternatively by making `require_live_*` treat a
//! delete/update of a reclaimed id as a no-op on the post-compaction path.

use std::collections::HashSet;

use selene_core::{EdgeId, NodeId};

use crate::error::{GraphError, GraphResult};
use crate::graph::{CompositePropertyIndexEntry, PropertyIndexEntry, SeleneGraph};
use crate::storage_compactor::{CompactionReport, LiveIdSet};
use crate::store::{EdgeStore, NodeStore, RowIndex};
use crate::typed_index::TypedIndex;

/// The product of compacting the CORE store.
pub struct CompactedCore {
    /// The dense, fully-rebuilt compacted graph (no dead/hole rows).
    pub graph: SeleneGraph,
    /// The surviving external ids — the cross-storage liveness set the snapshot
    /// publisher (4c) hands to downstream [`crate::StorageCompactor`]s.
    pub live: LiveIdSet,
    /// What CORE reclaimed.
    pub report: CompactionReport,
}

/// Compute the live external-id set from a graph's alive rows.
///
/// Reads the stable id from `row_to_id` per alive row (never `row + 1`); hole
/// rows resolve to `None` and are skipped.
#[must_use]
pub fn live_id_set(graph: &SeleneGraph) -> LiveIdSet {
    let mut live = LiveIdSet::new();
    live.nodes.reserve(graph.node_store.alive.len() as usize);
    live.edges.reserve(graph.edge_store.alive.len() as usize);
    for row in graph.node_store.alive.iter() {
        if let Some(id) = graph.node_id_for_row(RowIndex::new(row)) {
            live.nodes.insert(id);
        }
    }
    for row in graph.edge_store.alive.iter() {
        if let Some(id) = graph.edge_id_for_row(RowIndex::new(row)) {
            live.edges.insert(id);
        }
    }
    live
}

/// Compact `graph`: drop every dead / hole row, renumber rows dense (ascending
/// old-row order), and rebuild all derived state. External ids and the
/// allocator high-water marks are preserved verbatim.
///
/// # Errors
///
/// Returns [`GraphError`] if an alive row has no mapped external id (a corrupt
/// id↔row mapping) or if the rebuilt graph fails its consistency check.
pub fn compact_core(graph: &SeleneGraph) -> GraphResult<CompactedCore> {
    let mut nodes = NodeStore::new();
    let mut live_nodes: HashSet<NodeId> =
        HashSet::with_capacity(graph.node_store.alive.len() as usize);
    for old_row in graph.node_store.alive.iter() {
        let r = old_row as usize;
        let id = graph
            .node_id_for_row(RowIndex::new(old_row))
            .ok_or_else(|| GraphError::Inconsistent {
                reason: format!("alive node row {old_row} has no external id during compaction"),
            })?;
        let column_missing = |col: &str| GraphError::Inconsistent {
            reason: format!("node {col} column missing live row {old_row} during compaction"),
        };
        // Strict (fail-loud) reads, symmetric with the edge columns below: a
        // misaligned column on an alive row is corruption, not an empty row.
        nodes.labels.push(
            graph
                .node_store
                .labels
                .get(r)
                .cloned()
                .ok_or_else(|| column_missing("labels"))?,
        );
        nodes.properties.push(
            graph
                .node_store
                .properties
                .get(r)
                .cloned()
                .ok_or_else(|| column_missing("properties"))?,
        );
        nodes.row_to_id.push(id);
        live_nodes.insert(id);
    }
    let node_len = nodes.labels.len() as u32;
    for new_row in 0..node_len {
        nodes.alive.insert(new_row);
    }

    let mut edges = EdgeStore::new();
    let mut live_edges: HashSet<EdgeId> =
        HashSet::with_capacity(graph.edge_store.alive.len() as usize);
    for old_row in graph.edge_store.alive.iter() {
        let r = old_row as usize;
        let id = graph
            .edge_id_for_row(RowIndex::new(old_row))
            .ok_or_else(|| GraphError::Inconsistent {
                reason: format!("alive edge row {old_row} has no external id during compaction"),
            })?;
        let column_missing = |col: &str| GraphError::Inconsistent {
            reason: format!("edge {col} column missing live row {old_row} during compaction"),
        };
        let source = graph
            .edge_store
            .source
            .get(r)
            .copied()
            .ok_or_else(|| column_missing("source"))?;
        let target = graph
            .edge_store
            .target
            .get(r)
            .copied()
            .ok_or_else(|| column_missing("target"))?;
        // Defense-in-depth: a surviving edge must reference surviving nodes. The
        // mutator's delete_node cascade guarantees this for engine-produced
        // graphs; reject a dangling edge (only constructible by a raw column
        // build) rather than rebuild adjacency keyed by a vanished node.
        if !live_nodes.contains(&source) || !live_nodes.contains(&target) {
            return Err(GraphError::Inconsistent {
                reason: format!(
                    "edge {id} survives compaction but endpoint {source} or {target} does not"
                ),
            });
        }
        edges.label.push(
            graph
                .edge_store
                .label
                .get(r)
                .copied()
                .ok_or_else(|| column_missing("label"))?,
        );
        edges.source.push(source);
        edges.target.push(target);
        edges.properties.push(
            graph
                .edge_store
                .properties
                .get(r)
                .cloned()
                .ok_or_else(|| column_missing("properties"))?,
        );
        edges.row_to_id.push(id);
        live_edges.insert(id);
    }
    let edge_len = edges.label.len() as u32;
    for new_row in 0..edge_len {
        edges.alive.insert(new_row);
    }

    let report = CompactionReport {
        reclaimed_nodes: (graph.node_store.len() as u64).saturating_sub(u64::from(node_len)),
        reclaimed_edges: (graph.edge_store.len() as u64).saturating_sub(u64::from(edge_len)),
    };

    // Assemble the dense graph. meta is preserved VERBATIM — the monotonic
    // next_node_id/next_edge_id high-water marks must NOT be recomputed from the
    // shrunken row count, or a compacted-away external id could be reused
    // (violating D11/D22). bound_type + generation carry through unchanged.
    let mut dense = SeleneGraph::new(graph.meta.graph_id);
    dense.meta = graph.meta.clone();
    dense.node_store = nodes;
    dense.edge_store = edges;

    // Carry over property-index *registrations* (which (label, property) are
    // indexed + their kind/name) with fresh empty indexes; the rebuilders below
    // refill the row bitmaps from the dense columns. Cloning the source entries
    // directly would copy stale OLD-row bitmaps.
    for ((label, property), entry) in &graph.property_index {
        dense.property_index.insert(
            (*label, *property),
            PropertyIndexEntry::new(TypedIndex::new(entry.kind()), entry.name),
        );
    }
    for (key, entry) in &graph.composite_property_index {
        dense.composite_property_index.insert(
            key.clone(),
            CompositePropertyIndexEntry::new(
                crate::CompositeTypedIndex::new(entry.kinds()),
                entry.declared_properties.clone(),
                entry.name,
            ),
        );
    }

    // Rebuild every derived structure from the dense columns — the same chain
    // SharedGraph::from_graph_parts_and_snapshot uses on the recovery path.
    crate::shared::rebuild_derived_state(&mut dense)?;
    crate::property_index::rebuild_property_indexes(&mut dense)?;
    crate::composite_property_index::rebuild_composite_property_indexes(&mut dense)?;

    // Debug-only structural net (matches the snapshot-load publication seam):
    // re-derive every index from the compacted columns and confirm agreement.
    #[cfg(debug_assertions)]
    if let Err(reason) = dense.assert_indexes_consistent() {
        return Err(GraphError::Inconsistent {
            reason: format!("compacted graph failed consistency check: {reason}"),
        });
    }

    // Single source of truth: the live set IS the ids the compaction kept, so
    // the LiveIdSet 4c hands to downstream compactors cannot drift from what the
    // dense graph holds.
    let live = LiveIdSet {
        nodes: live_nodes,
        edges: live_edges,
    };
    Ok(CompactedCore {
        graph: dense,
        live,
        report,
    })
}

#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod tests;
