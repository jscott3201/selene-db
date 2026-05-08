//! Typed mutation funnel per spec 03 section 4.3.

use std::collections::BTreeSet;
use std::sync::Arc;

use roaring::RoaringBitmap;
use selene_core::{
    Change, EdgeId, GraphId, IStr, LabelDiff, LabelSet, NodeId, Origin, PropertyDiff, PropertyMap,
    SchemaChange,
};

use crate::adjacency::{AdjacencyEdge, AdjacencyEntry};
use crate::error::{GraphError, GraphResult};
use crate::store::{edge_row_index, node_row_index};
use crate::write_txn::WriteTxn;

/// Borrowed mutation builder for one write transaction.
pub struct Mutator<'tx, 'g> {
    txn: &'tx mut WriteTxn<'g>,
    _origin: Origin,
}

impl<'tx, 'g> Mutator<'tx, 'g> {
    pub(crate) fn new(txn: &'tx mut WriteTxn<'g>, origin: Origin) -> Self {
        Self {
            txn,
            _origin: origin,
        }
    }

    /// Create a node, emit `Change::NodeCreated`, and return its ID.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::IdOverflow`] when the allocator advances past the
    /// v1 row-index range (max 2^32 rows).
    pub fn create_node(&mut self, labels: LabelSet, props: PropertyMap) -> GraphResult<NodeId> {
        let id = self.txn.allocator.allocate_node();
        let row = node_row_index(id).ok_or_else(|| GraphError::IdOverflow {
            kind: "node",
            raw: id.get(),
            max: u32::MAX as u64 + 1,
        })? as usize;
        ensure_node_rows(&mut self.txn.working, row);
        if row == self.txn.working.node_store.len() {
            self.txn.working.node_store.labels.push(labels.clone());
            self.txn.working.node_store.properties.push(props.clone());
        } else {
            self.txn.working.node_store.labels.set(row, labels.clone());
            self.txn
                .working
                .node_store
                .properties
                .set(row, props.clone());
        }
        self.txn.working.node_store.alive.insert(row as u32);
        insert_node_labels(&mut self.txn.working.idx_label, row as u32, &labels);
        self.txn.changes.push(Change::NodeCreated {
            id,
            labels,
            properties: props,
        });
        Ok(id)
    }

    /// Create an edge between two alive nodes.
    pub fn create_edge(
        &mut self,
        label: IStr,
        source: NodeId,
        target: NodeId,
        props: PropertyMap,
    ) -> GraphResult<EdgeId> {
        self.require_live_node(source)?;
        self.require_live_node(target)?;
        let id = self.txn.allocator.allocate_edge();
        let row = edge_row_index(id).ok_or_else(|| GraphError::IdOverflow {
            kind: "edge",
            raw: id.get(),
            max: u32::MAX as u64 + 1,
        })? as usize;
        ensure_edge_rows(&mut self.txn.working, row)?;
        if row == self.txn.working.edge_store.len() {
            self.txn.working.edge_store.label.push(label);
            self.txn.working.edge_store.source.push(source);
            self.txn.working.edge_store.target.push(target);
            self.txn.working.edge_store.properties.push(props.clone());
        } else {
            self.txn.working.edge_store.label.set(row, label);
            self.txn.working.edge_store.source.set(row, source);
            self.txn.working.edge_store.target.set(row, target);
            self.txn
                .working
                .edge_store
                .properties
                .set(row, props.clone());
        }
        self.txn.working.edge_store.alive.insert(row as u32);
        insert_index_row(&mut self.txn.working.idx_edge_label, label, row as u32);

        self.txn
            .working
            .adjacency_out
            .entry(source)
            .or_default()
            .add(AdjacencyEdge {
                label,
                neighbor: target,
                edge_id: id,
            });
        self.txn
            .working
            .adjacency_in
            .entry(target)
            .or_default()
            .add(AdjacencyEdge {
                label,
                neighbor: source,
                edge_id: id,
            });
        self.txn.changes.push(Change::EdgeCreated {
            id,
            label,
            source,
            target,
            properties: props,
        });
        Ok(id)
    }

    /// Update an alive node and emit `Change::NodeUpdated`.
    pub fn update_node(
        &mut self,
        id: NodeId,
        labels_diff: LabelDiff,
        props_diff: PropertyDiff,
    ) -> GraphResult<()> {
        let row = self.require_live_node(id)?;

        // Compute the new label set without mutating the working graph yet.
        let mut labels = self
            .txn
            .working
            .node_store
            .labels
            .get(row)
            .cloned()
            .unwrap_or_default();
        for label in labels_diff.added.iter().copied() {
            labels.insert(label);
        }
        for label in labels_diff.removed.iter() {
            labels.remove(label);
        }

        // Apply the property diff up front; if it errors we leave the working
        // graph (including idx_label) untouched so the transaction can still
        // be safely rolled back or aborted without leaking inconsistent state.
        let mut props = self
            .txn
            .working
            .node_store
            .properties
            .get(row)
            .cloned()
            .unwrap_or_default();
        apply_property_diff(&mut props, &props_diff)?;

        // Now atomic in the working graph: write columns, then update indexes.
        self.txn.working.node_store.labels.set(row, labels);
        self.txn.working.node_store.properties.set(row, props);
        for label in labels_diff.added.iter().copied() {
            insert_index_row(&mut self.txn.working.idx_label, label, row as u32);
        }
        for label in labels_diff.removed.iter() {
            remove_index_row(&mut self.txn.working.idx_label, label, row as u32);
        }

        self.txn.changes.push(Change::NodeUpdated {
            id,
            labels_diff,
            properties_diff: props_diff,
        });
        Ok(())
    }

    /// Update an alive edge and emit `Change::EdgeUpdated`.
    ///
    /// Edge labels are immutable, so property updates do not touch
    /// `idx_edge_label`.
    pub fn update_edge(&mut self, id: EdgeId, props_diff: PropertyDiff) -> GraphResult<()> {
        let row = self.require_live_edge(id)?;
        let mut props = self
            .txn
            .working
            .edge_store
            .properties
            .get(row)
            .cloned()
            .unwrap_or_default();
        apply_property_diff(&mut props, &props_diff)?;
        self.txn.working.edge_store.properties.set(row, props);
        self.txn.changes.push(Change::EdgeUpdated {
            id,
            properties_diff: props_diff,
        });
        Ok(())
    }

    /// Delete an alive node and cascade delete incident edges.
    pub fn delete_node(&mut self, id: NodeId) -> GraphResult<()> {
        let row = self.require_live_node(id)?;
        let labels = self
            .txn
            .working
            .node_store
            .labels
            .get(row)
            .cloned()
            .unwrap_or_default();
        let mut incident = BTreeSet::new();
        if let Some(outgoing) = self.txn.working.adjacency_out.get(&id) {
            incident.extend(outgoing.iter().map(|edge| edge.edge_id));
        }
        if let Some(incoming) = self.txn.working.adjacency_in.get(&id) {
            incident.extend(incoming.iter().map(|edge| edge.edge_id));
        }
        remove_node_labels(&mut self.txn.working.idx_label, row as u32, &labels);
        self.txn.working.node_store.alive.remove(row as u32);
        self.txn.changes.push(Change::NodeDeleted { id });
        for edge_id in incident {
            self.delete_edge_inner(edge_id, true)?;
        }
        Ok(())
    }

    /// Delete an alive edge.
    pub fn delete_edge(&mut self, id: EdgeId) -> GraphResult<()> {
        self.delete_edge_inner(id, true)
    }

    /// Append a schema-change WAL payload.
    ///
    /// This is a pass-through accumulator in BRIEF-07. Catalog graph mutation
    /// and closed-graph validation are intentionally deferred.
    pub fn schema_change(&mut self, graph: GraphId, change: SchemaChange) {
        self.txn
            .changes
            .push(Change::SchemaChanged { graph, change });
    }

    /// Append an opaque extension-provider event.
    ///
    /// Providers are not registered in BRIEF-07; replay is owned by future
    /// index-provider work.
    pub fn extension_event(&mut self, provider: IStr, payload: Arc<[u8]>) {
        self.txn
            .changes
            .push(Change::IndexExtensionEvent { provider, payload });
    }

    /// Borrow the transaction-local working graph.
    #[must_use]
    pub fn read(&self) -> &crate::SeleneGraph {
        &self.txn.working
    }

    fn delete_edge_inner(&mut self, id: EdgeId, record_change: bool) -> GraphResult<()> {
        let row = self.require_live_edge(id)?;
        let label = *self
            .txn
            .working
            .edge_store
            .label
            .get(row)
            .ok_or(GraphError::EdgeNotFound { id })?;
        let source = *self
            .txn
            .working
            .edge_store
            .source
            .get(row)
            .ok_or(GraphError::EdgeNotFound { id })?;
        let target = *self
            .txn
            .working
            .edge_store
            .target
            .get(row)
            .ok_or(GraphError::EdgeNotFound { id })?;
        self.txn.working.edge_store.alive.remove(row as u32);
        remove_index_row(&mut self.txn.working.idx_edge_label, &label, row as u32);
        if let Some(mut entry) = self.txn.working.adjacency_out.get(&source).cloned() {
            entry.remove(id);
            update_or_remove_entry(&mut self.txn.working.adjacency_out, source, entry);
        }
        if let Some(mut entry) = self.txn.working.adjacency_in.get(&target).cloned() {
            entry.remove(id);
            update_or_remove_entry(&mut self.txn.working.adjacency_in, target, entry);
        }
        if record_change {
            self.txn.changes.push(Change::EdgeDeleted { id });
        }
        Ok(())
    }

    fn require_live_node(&self, id: NodeId) -> GraphResult<usize> {
        let row = node_row_index(id).ok_or(GraphError::NodeNotFound { id })?;
        if row as usize >= self.txn.working.node_store.len() {
            return Err(GraphError::NodeNotFound { id });
        }
        if !self.txn.working.node_store.is_alive(row) {
            return Err(GraphError::NodeNotAlive { id });
        }
        Ok(row as usize)
    }

    fn require_live_edge(&self, id: EdgeId) -> GraphResult<usize> {
        let row = edge_row_index(id).ok_or(GraphError::EdgeNotFound { id })?;
        if row as usize >= self.txn.working.edge_store.len() {
            return Err(GraphError::EdgeNotFound { id });
        }
        if !self.txn.working.edge_store.is_alive(row) {
            return Err(GraphError::EdgeNotAlive { id });
        }
        Ok(row as usize)
    }
}

fn ensure_node_rows(graph: &mut crate::SeleneGraph, target_row: usize) {
    while graph.node_store.len() < target_row {
        graph.node_store.labels.push(LabelSet::new());
        graph.node_store.properties.push(PropertyMap::new());
    }
}

fn ensure_edge_rows(graph: &mut crate::SeleneGraph, target_row: usize) -> GraphResult<()> {
    if graph.edge_store.len() >= target_row {
        return Ok(());
    }
    let hole_label = edge_hole_label()?;
    while graph.edge_store.len() < target_row {
        graph.edge_store.label.push(hole_label);
        graph.edge_store.source.push(NodeId::TOMBSTONE);
        graph.edge_store.target.push(NodeId::TOMBSTONE);
        graph.edge_store.properties.push(PropertyMap::new());
    }
    Ok(())
}

/// Cache the sentinel label used to pad over aborted-tx EdgeId holes.
///
/// First call interns `"__selene_hole"`; subsequent calls return the cached
/// `IStr` so transaction-time hole materialization never re-hits the interner.
/// If interner capacity is exhausted on the first call, the error propagates
/// to the caller as a typed `CoreError::IStrCapExceeded`.
fn edge_hole_label() -> selene_core::CoreResult<IStr> {
    static CELL: std::sync::OnceLock<IStr> = std::sync::OnceLock::new();
    if let Some(value) = CELL.get() {
        return Ok(*value);
    }
    let value = selene_core::intern("__selene_hole")?;
    let _ = CELL.set(value);
    Ok(value)
}

fn insert_node_labels(index: &mut imbl::HashMap<IStr, RoaringBitmap>, row: u32, labels: &LabelSet) {
    for label in labels.iter().copied() {
        insert_index_row(index, label, row);
    }
}

fn remove_node_labels(index: &mut imbl::HashMap<IStr, RoaringBitmap>, row: u32, labels: &LabelSet) {
    for label in labels.iter() {
        remove_index_row(index, label, row);
    }
}

fn insert_index_row(index: &mut imbl::HashMap<IStr, RoaringBitmap>, label: IStr, row: u32) {
    let mut bitmap = index.get(&label).cloned().unwrap_or_default();
    bitmap.insert(row);
    index.insert(label, bitmap);
}

fn remove_index_row(index: &mut imbl::HashMap<IStr, RoaringBitmap>, label: &IStr, row: u32) {
    if let Some(mut bitmap) = index.get(label).cloned() {
        bitmap.remove(row);
        if bitmap.is_empty() {
            index.remove(label);
        } else {
            index.insert(*label, bitmap);
        }
    }
}

fn apply_property_diff(map: &mut PropertyMap, diff: &PropertyDiff) -> GraphResult<()> {
    for (key, value) in diff.set.iter() {
        map.set(*key, value.clone())?;
    }
    for key in diff.removed.iter() {
        map.remove(key);
    }
    Ok(())
}

fn update_or_remove_entry(
    map: &mut imbl::HashMap<NodeId, AdjacencyEntry>,
    id: NodeId,
    entry: AdjacencyEntry,
) {
    if entry.is_empty() {
        map.remove(&id);
    } else {
        map.insert(id, entry);
    }
}

#[cfg(test)]
mod tests;
