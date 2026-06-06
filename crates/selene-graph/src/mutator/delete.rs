use std::collections::BTreeSet;

use selene_core::{Change, DbString, EdgeId, LabelSet, NodeId, PropertyMap, db_string};

use super::{Mutator, remove_index_row, remove_node_labels};
use crate::adjacency::AdjacencyEntry;
use crate::error::{GraphError, GraphResult};
use crate::store::RowIndex;

impl<'tx, 'g> Mutator<'tx, 'g> {
    /// Delete an alive node and cascade delete incident edges.
    pub fn delete_node(&mut self, id: NodeId) -> GraphResult<()> {
        let row = self.require_live_node(id)?;
        let incident = self.remove_node_row(id, row)?;
        self.txn.changes.push(Change::NodeDeleted { id });
        for edge_id in incident {
            self.delete_edge_inner(edge_id, true)?;
        }
        Ok(())
    }

    /// Remove one node row from every in-memory structure (label index,
    /// property/composite indexes, liveness) and return its incident edges.
    ///
    /// This is the change-free removal core shared by [`Self::delete_node`] and
    /// [`Self::truncate_node_type`]; callers own the changeset accounting (one
    /// `NodeDeleted` for DETACH DELETE, one declarative truncate change plus
    /// staged per-row tombstones for TRUNCATE). The returned incident set spans
    /// edges of **every** edge type touching the node (derived from both
    /// adjacency directions) so no dangling edge can survive.
    pub(super) fn remove_node_row(
        &mut self,
        id: NodeId,
        row: usize,
    ) -> GraphResult<BTreeSet<EdgeId>> {
        let graph = self.txn.read();
        let labels = graph
            .node_store
            .labels
            .get(row)
            .cloned()
            .unwrap_or_default();
        let props = graph
            .node_store
            .properties
            .get(row)
            .cloned()
            .unwrap_or_default();
        let mut incident = BTreeSet::new();
        if let Some(outgoing) = graph.adjacency_out.get(&id) {
            incident.extend(outgoing.iter().map(|edge| edge.edge_id));
        }
        if let Some(incoming) = graph.adjacency_in.get(&id) {
            incident.extend(incoming.iter().map(|edge| edge.edge_id));
        }
        {
            let graph = self.txn.guard_mut();
            remove_node_labels(&mut graph.idx_label, row as u32, &labels);
            crate::property_index::apply_node_delete(
                &mut graph.property_index,
                &labels,
                &props,
                row as u32,
            )?;
            crate::composite_property_index::apply_node_delete(
                &mut graph.composite_property_index,
                &labels,
                &props,
                row as u32,
            )?;
            crate::vector_index::apply_node_delete(
                &mut graph.vector_index,
                &labels,
                &props,
                row as u32,
            )?;
            crate::text_index::apply_node_delete(
                &mut graph.text_index,
                &labels,
                &props,
                row as u32,
                id,
            );
            graph.node_store.labels.set(row, LabelSet::new());
            graph.node_store.properties.set(row, PropertyMap::new());
            graph.node_store.alive.remove(row as u32);
            // BRIEF-Item-4a: KEEP the real external id in row_to_id for the now
            // dead row (and keep the id -> row map entry). A deleted id stays
            // resolvable -> its dead row -> NodeNotAlive, identically across the
            // live and recovery paths: the snapshot persists dead rows with their
            // id (sections.rs) and STEP 9 encodes that id from this column. Only
            // never-committed aborted-tx hole rows carry NodeId::TOMBSTONE
            // (-> None -> NotFound, the accepted refinement).

            // GRAPH-05: drop this node's own adjacency entries wholesale, O(1)
            // each. The incident set was already captured above, so the
            // per-edge cascade (`remove_edge_row`) only has to clear the
            // *neighbor* side of each incident edge; a `get_mut` on this
            // now-absent hub key no-ops. This is what turns a degree-`D` hub
            // delete from O(D^2) (clone + linear-scan the shrinking hub entry
            // once per incident edge) into O(D).
            graph.adjacency_out.remove(&id);
            graph.adjacency_in.remove(&id);
        }
        Ok(incident)
    }

    /// Delete an alive edge.
    pub fn delete_edge(&mut self, id: EdgeId) -> GraphResult<()> {
        self.delete_edge_inner(id, true)
    }

    /// Remove every node carrying `label` and all of their incident edges in one
    /// declarative truncate.
    ///
    /// Observationally identical to `MATCH (n:L) DETACH DELETE n`: every matched
    /// node and every incident edge (of **any** edge type, derived from both
    /// adjacency directions) is removed via the same change-free in-memory path
    /// `delete_node`/`delete_edge_inner` use, so the resulting graph state is
    /// byte-identical. The difference is the changeset: exactly **one**
    /// declarative [`Change::NodesOfTypeTruncated`] is recorded regardless of the
    /// number of rows removed (O(1) WAL write, deletion-reclamation audit
    /// Item 11), while the per-row `NodeDeleted`/`EdgeDeleted` tombstones are
    /// staged for provider/subscriber fan-out so derived state (e.g. extension
    /// providers) is reclaimed without leaks. An absent label is a clean no-op
    /// (no change is recorded), matching DETACH DELETE of zero matches; a second
    /// truncate of the same label is therefore idempotent.
    ///
    /// All logic lives in the mutator (the single write funnel, hard rule 11) so
    /// future `DROP NODE TYPE CASCADE` and `DROP GRAPH` factory-reset paths can
    /// reuse it without an N+1 change storm.
    pub fn truncate_node_type(&mut self, label: DbString) -> GraphResult<()> {
        // Snapshot the matched node rows and derive every incident edge BEFORE
        // any removal, exactly as delete_node does — removal mutates the
        // adjacency/label structures we are iterating.
        let matched_rows: Vec<u32> = match self.txn.read().nodes_with_label(&label) {
            Some(bitmap) => bitmap.iter().collect(),
            None => return Ok(()),
        };
        if matched_rows.is_empty() {
            return Ok(());
        }
        let mut node_tombstones = Vec::with_capacity(matched_rows.len());
        let mut incident_edges = BTreeSet::new();
        for row in matched_rows {
            // Skip rows that are not alive (defensive: idx_label is kept in
            // lockstep with liveness, but a dead row must never be re-removed).
            if !self.txn.read().node_store.is_alive(row) {
                continue;
            }
            let Some(id) = self.txn.read().node_id_for_row(RowIndex::new(row)) else {
                continue;
            };
            incident_edges.append(&mut self.remove_node_row(id, row as usize)?);
            node_tombstones.push(Change::NodeDeleted { id });
        }
        if node_tombstones.is_empty() {
            return Ok(());
        }
        let mut expansion = node_tombstones;
        for edge_id in incident_edges {
            let row = self
                .txn
                .read()
                .row_for_edge_id(edge_id)
                .ok_or(GraphError::EdgeNotFound { id: edge_id })?
                .get();
            // An incident edge may already be gone if two truncated endpoints
            // shared it; remove_edge_row is only called for still-alive rows.
            if self.txn.read().edge_store.is_alive(row) {
                self.remove_edge_row(edge_id, row as usize)?;
                expansion.push(Change::EdgeDeleted { id: edge_id });
            }
        }
        let index = self.txn.changes.len();
        self.txn
            .changes
            .push(Change::NodesOfTypeTruncated { label });
        self.txn.truncate_expansions.push((index, expansion));
        Ok(())
    }

    /// Remove every edge carrying `label` in one declarative truncate.
    ///
    /// Observationally identical to `MATCH ()-[e:L]->() DELETE e`: each matched
    /// edge is removed via the same change-free path `delete_edge` uses, leaving
    /// the graph dangling-free. Records exactly **one** declarative
    /// [`Change::EdgesOfTypeTruncated`] (O(1) WAL) and stages per-row
    /// `EdgeDeleted` tombstones for fan-out. Absent label is a clean idempotent
    /// no-op.
    pub fn truncate_edge_type(&mut self, label: DbString) -> GraphResult<()> {
        let matched_rows: Vec<u32> = match self.txn.read().edges_with_label(&label) {
            Some(bitmap) => bitmap.iter().collect(),
            None => return Ok(()),
        };
        if matched_rows.is_empty() {
            return Ok(());
        }
        let mut expansion = Vec::with_capacity(matched_rows.len());
        for row in matched_rows {
            if !self.txn.read().edge_store.is_alive(row) {
                continue;
            }
            let Some(id) = self.txn.read().edge_id_for_row(RowIndex::new(row)) else {
                continue;
            };
            self.remove_edge_row(id, row as usize)?;
            expansion.push(Change::EdgeDeleted { id });
        }
        if expansion.is_empty() {
            return Ok(());
        }
        let index = self.txn.changes.len();
        self.txn
            .changes
            .push(Change::EdgesOfTypeTruncated { label });
        self.txn.truncate_expansions.push((index, expansion));
        Ok(())
    }

    fn delete_edge_inner(&mut self, id: EdgeId, record_change: bool) -> GraphResult<()> {
        let row = self.require_live_edge(id)?;
        self.remove_edge_row(id, row)?;
        if record_change {
            self.txn.changes.push(Change::EdgeDeleted { id });
        }
        Ok(())
    }

    /// Remove one edge row from every in-memory structure (liveness, edge-label
    /// index, both adjacency directions) without recording any change.
    ///
    /// Shared change-free core for [`Self::delete_edge_inner`] and the truncate
    /// paths; callers own changeset accounting.
    pub(super) fn remove_edge_row(&mut self, id: EdgeId, row: usize) -> GraphResult<()> {
        let graph = self.txn.read();
        let label = graph
            .edge_store
            .label
            .get(row)
            .cloned()
            .ok_or(GraphError::EdgeNotFound { id })?;
        let source = *graph
            .edge_store
            .source
            .get(row)
            .ok_or(GraphError::EdgeNotFound { id })?;
        let target = *graph
            .edge_store
            .target
            .get(row)
            .ok_or(GraphError::EdgeNotFound { id })?;
        let graph = self.txn.guard_mut();
        graph.edge_store.alive.remove(row as u32);
        // BRIEF-Item-4a: keep the real id in row_to_id for the dead row (see
        // remove_node_row); only never-committed holes carry EdgeId::TOMBSTONE.
        remove_index_row(&mut graph.idx_edge_label, &label, row as u32);
        // GRAPH-05: remove the edge from each endpoint's adjacency entry in
        // place (no full-`SmallVec` clone), dropping the map key only when the
        // entry becomes empty. In a node-delete cascade the hub endpoint's
        // entry has already been dropped wholesale by `remove_node_row`, so its
        // lookup no-ops and only the neighbor side is touched.
        remove_edge_from_adjacency(&mut graph.adjacency_out, source, id);
        remove_edge_from_adjacency(&mut graph.adjacency_in, target, id);
        graph.edge_store.label.set(row, db_string("")?);
        graph.edge_store.source.set(row, NodeId::TOMBSTONE);
        graph.edge_store.target.set(row, NodeId::TOMBSTONE);
        graph.edge_store.properties.set(row, PropertyMap::new());
        Ok(())
    }
}

/// Remove edge `edge_id` from one direction's adjacency `map` in place,
/// dropping the node's entry only when it becomes empty.
///
/// In-place via `imbl::HashMap::get_mut` (no full-`SmallVec` clone), so a
/// degree-`D` hub-delete cascade is O(D) rather than O(D^2). A missing key is a
/// no-op — e.g. the hub endpoint whose whole entry `remove_node_row` already
/// dropped. The empty-key removal preserves the "no present-but-empty entry"
/// invariant asserted by the consistency checks.
fn remove_edge_from_adjacency(
    map: &mut imbl::HashMap<NodeId, AdjacencyEntry>,
    node: NodeId,
    edge_id: EdgeId,
) {
    // The `get_mut` borrow ends with the match expression (it yields a bool),
    // so the conditional `remove` below is a fresh, non-overlapping borrow.
    let now_empty = match map.get_mut(&node) {
        Some(entry) => {
            entry.remove(edge_id);
            entry.is_empty()
        }
        None => false,
    };
    if now_empty {
        map.remove(&node);
    }
}
