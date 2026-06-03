//! CORE graph compaction mechanism (BRIEF-Item-4b / 4c).
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
//! **Cross-epoch WAL replay (BRIEF-Item-4e — RESOLVED, no new mechanism).**
//! Compaction drops dead rows, so a *deleted* external id that gets compacted
//! away resolves `NotFound` afterwards (was `NotAlive` under 4a's Option B). The
//! concern was that a `Change::NodeDeleted` / `EdgeDeleted` written *before* a
//! compaction, replayed *after* loading the compacted snapshot, would route
//! through `require_live_*` (`recovery_state`) and hard-error on the reclaimed
//! id. This cannot happen in the normal flow: a snapshot is published via
//! `WalWriter::rotate_with_manifest`, which both advances the MANIFEST
//! `live_snapshot_seq` WAL floor AND physically truncates the WAL (`set_len(0)`
//! then a fresh header). Pre-compaction entries are therefore *gone* AND below
//! the recovery floor (`recovery.rs` replays only `header.sequence > floor`) —
//! they can never be replayed against a compacted snapshot. The only cross-epoch
//! replay is of *post*-snapshot entries, which resolve against the dense rows:
//! a post-compaction `NodeCreated` appends (BRIEF-Item-4c), and a post-compaction
//! `NodeDeleted` of a survivor finds it in the compacted snapshot (proven by
//! `recover_tests::nodeid_split_recovery`). The `require_live_*` hard-error is
//! deliberately *retained* as genuine-corruption detection — a "no-op the
//! reclaimed-id delete" alternative was rejected because it would mask a truly
//! inconsistent WAL. The MANIFEST `compaction_epoch` field stays reserved (`0`).

use std::collections::HashSet;

use selene_core::NodeId;

use crate::error::{GraphError, GraphResult};
use crate::graph::{
    CompositePropertyIndexEntry, PropertyIndexEntry, SeleneGraph, VectorIndexEntry,
};
use crate::store::{EdgeStore, NodeStore, RowIndex};
use crate::typed_index::TypedIndex;

/// What CORE reclaimed during a compaction pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompactionReport {
    /// Node rows (dead + hole) dropped.
    pub reclaimed_nodes: u64,
    /// Edge rows (dead + hole) dropped.
    pub reclaimed_edges: u64,
}

/// The product of compacting the CORE store.
pub struct CompactedCore {
    /// The dense, fully-rebuilt compacted graph (no dead/hole rows).
    pub graph: SeleneGraph,
    /// What CORE reclaimed.
    pub report: CompactionReport,
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
                .cloned()
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
            (label.clone(), property.clone()),
            PropertyIndexEntry::new(TypedIndex::new(entry.kind()), entry.name.clone()),
        );
    }
    for (key, entry) in &graph.composite_property_index {
        dense.composite_property_index.insert(
            key.clone(),
            CompositePropertyIndexEntry::new(
                crate::CompositeTypedIndex::new(entry.kinds()),
                entry.declared_properties.clone(),
                entry.name.clone(),
            ),
        );
    }
    for ((label, property), entry) in &graph.vector_index {
        dense.vector_index.insert(
            (label.clone(), property.clone()),
            VectorIndexEntry::new(
                crate::VectorIndex::new(entry.kind(), entry.dimension())?,
                entry.name.clone(),
            ),
        );
    }

    // Rebuild every derived structure from the dense columns — the same chain
    // SharedGraph::from_graph_parts_and_snapshot uses on the recovery path.
    crate::shared::rebuild_derived_state(&mut dense)?;
    crate::property_index::rebuild_property_indexes(&mut dense)?;
    crate::composite_property_index::rebuild_composite_property_indexes(&mut dense)?;
    crate::vector_index::rebuild_vector_indexes(&mut dense)?;

    // Debug-only structural net (matches the snapshot-load publication seam):
    // re-derive every index from the compacted columns and confirm agreement.
    #[cfg(debug_assertions)]
    if let Err(reason) = dense.assert_indexes_consistent() {
        return Err(GraphError::Inconsistent {
            reason: format!("compacted graph failed consistency check: {reason}"),
        });
    }

    Ok(CompactedCore {
        graph: dense,
        report,
    })
}

#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod tests;
