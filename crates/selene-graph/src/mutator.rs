//! Typed mutation funnel per spec 03 section 4.3.

use std::collections::BTreeSet;
use std::sync::Arc;

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
    #[must_use]
    pub fn create_node(&mut self, labels: LabelSet, props: PropertyMap) -> NodeId {
        let id = self.txn.allocator.allocate_node();
        let row = node_row_index(id).expect("node id exceeds v1 row index range") as usize;
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
        self.txn.changes.push(Change::NodeCreated {
            id,
            labels,
            properties: props,
        });
        id
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
        let row = edge_row_index(id).expect("edge id exceeds v1 row index range") as usize;
        ensure_edge_rows(&mut self.txn.working, row);
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
        let mut props = self
            .txn
            .working
            .node_store
            .properties
            .get(row)
            .cloned()
            .unwrap_or_default();
        apply_property_diff(&mut props, &props_diff)?;
        self.txn.working.node_store.labels.set(row, labels);
        self.txn.working.node_store.properties.set(row, props);
        self.txn.changes.push(Change::NodeUpdated {
            id,
            labels_diff,
            properties_diff: props_diff,
        });
        Ok(())
    }

    /// Update an alive edge and emit `Change::EdgeUpdated`.
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
        let mut incident = BTreeSet::new();
        if let Some(outgoing) = self.txn.working.adjacency_out.get(&id) {
            incident.extend(outgoing.iter().map(|edge| edge.edge_id));
        }
        if let Some(incoming) = self.txn.working.adjacency_in.get(&id) {
            incident.extend(incoming.iter().map(|edge| edge.edge_id));
        }
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
        if let Some(mut entry) = self.txn.working.adjacency_out.get(&source).cloned() {
            entry.remove(id);
            update_or_remove_entry(&mut self.txn.working.adjacency_out, source, entry);
        }
        if let Some(mut entry) = self.txn.working.adjacency_in.get(&target).cloned() {
            entry.remove(id);
            update_or_remove_entry(&mut self.txn.working.adjacency_in, target, entry);
        }
        let _ = label;
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

fn ensure_edge_rows(graph: &mut crate::SeleneGraph, target_row: usize) {
    while graph.edge_store.len() < target_row {
        graph
            .edge_store
            .label
            .push(selene_core::intern("__selene_hole").unwrap());
        graph.edge_store.source.push(NodeId::TOMBSTONE);
        graph.edge_store.target.push(NodeId::TOMBSTONE);
        graph.edge_store.properties.push(PropertyMap::new());
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
mod tests {
    use proptest::prelude::*;
    use selene_core::{Change, GraphId, PredefinedValueType, Value, ValueType, intern};

    use super::*;
    use crate::SharedGraph;

    fn empty_node(mutator: &mut Mutator<'_, '_>) -> NodeId {
        mutator.create_node(LabelSet::new(), PropertyMap::new())
    }

    #[test]
    fn create_node_returns_id_and_emits_change() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let id = {
            let mut mutator = txn.mutator();
            mutator.create_node(LabelSet::new(), PropertyMap::new())
        };
        let outcome = txn.commit().unwrap();
        assert_eq!(id, NodeId::new(1));
        assert!(
            matches!(outcome.changes[0], Change::NodeCreated { id, .. } if id == NodeId::new(1))
        );
    }

    #[test]
    fn create_edge_with_invalid_source_fails() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let target = empty_node(&mut mutator);
        let err = mutator
            .create_edge(
                intern("edge.invalid.source").unwrap(),
                NodeId::new(99),
                target,
                PropertyMap::new(),
            )
            .unwrap_err();
        assert!(matches!(err, GraphError::NodeNotFound { id } if id == NodeId::new(99)));
    }

    #[test]
    fn create_edge_with_invalid_target_fails() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let source = empty_node(&mut mutator);
        let err = mutator
            .create_edge(
                intern("edge.invalid.target").unwrap(),
                source,
                NodeId::new(99),
                PropertyMap::new(),
            )
            .unwrap_err();
        assert!(matches!(err, GraphError::NodeNotFound { id } if id == NodeId::new(99)));
    }

    #[test]
    fn update_node_with_unknown_id_fails() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let err = mutator
            .update_node(
                NodeId::new(1),
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap_err();
        assert!(matches!(err, GraphError::NodeNotFound { .. }));
    }

    #[test]
    fn delete_node_cascades_to_incident_edges() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let (a, b, edge) = {
            let mut mutator = txn.mutator();
            let a = empty_node(&mut mutator);
            let b = empty_node(&mut mutator);
            let edge = mutator
                .create_edge(intern("edge.cascade").unwrap(), a, b, PropertyMap::new())
                .unwrap();
            mutator.delete_node(a).unwrap();
            (a, b, edge)
        };
        txn.commit().unwrap();
        let snapshot = shared.read();
        assert!(!snapshot.is_node_alive(a));
        assert!(snapshot.is_node_alive(b));
        assert!(!snapshot.is_edge_alive(edge));
        assert!(snapshot.incoming_edges(b).is_none());
    }

    #[test]
    fn delete_edge_updates_both_adjacencies() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let (a, b, edge) = {
            let mut mutator = txn.mutator();
            let a = empty_node(&mut mutator);
            let b = empty_node(&mut mutator);
            let edge = mutator
                .create_edge(intern("edge.delete").unwrap(), a, b, PropertyMap::new())
                .unwrap();
            mutator.delete_edge(edge).unwrap();
            (a, b, edge)
        };
        txn.commit().unwrap();
        let snapshot = shared.read();
        assert!(!snapshot.is_edge_alive(edge));
        assert!(snapshot.outgoing_edges(a).is_none());
        assert!(snapshot.incoming_edges(b).is_none());
    }

    #[test]
    fn read_within_tx_sees_own_writes() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let id = empty_node(&mut mutator);
        assert!(mutator.read().is_node_alive(id));
    }

    #[test]
    fn multi_step_tx_emits_changes_in_order() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let id = {
            let mut mutator = txn.mutator();
            let id = empty_node(&mut mutator);
            mutator
                .update_node(
                    id,
                    LabelDiff::new([intern("node.updated").unwrap()], []).unwrap(),
                    PropertyDiff::new([], []).unwrap(),
                )
                .unwrap();
            mutator.delete_node(id).unwrap();
            id
        };
        let outcome = txn.commit().unwrap();
        assert!(matches!(outcome.changes[0], Change::NodeCreated { .. }));
        assert!(matches!(outcome.changes[1], Change::NodeUpdated { .. }));
        assert_eq!(outcome.changes[2], Change::NodeDeleted { id });
    }

    #[test]
    fn extension_event_emits_change_passthrough() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator.extension_event(intern("provider").unwrap(), Arc::from([1_u8, 2]));
        }
        let outcome = txn.commit().unwrap();
        assert!(matches!(
            outcome.changes[0],
            Change::IndexExtensionEvent { .. }
        ));
    }

    #[test]
    fn schema_change_emits_change_passthrough() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator.schema_change(
                GraphId::new(1),
                SchemaChange::GraphDropped {
                    id: GraphId::new(2),
                },
            );
        }
        let outcome = txn.commit().unwrap();
        assert!(matches!(outcome.changes[0], Change::SchemaChanged { .. }));
    }

    #[test]
    fn update_edge_updates_properties() {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let (edge, prop) = {
            let mut mutator = txn.mutator();
            let a = empty_node(&mut mutator);
            let b = empty_node(&mut mutator);
            let edge = mutator
                .create_edge(intern("edge.update").unwrap(), a, b, PropertyMap::new())
                .unwrap();
            let prop = intern("edge.prop").unwrap();
            mutator
                .update_edge(
                    edge,
                    PropertyDiff::new([(prop, Value::String(prop))], []).unwrap(),
                )
                .unwrap();
            (edge, prop)
        };
        txn.commit().unwrap();
        assert_eq!(
            shared.read().edge_properties(edge).unwrap().get(&prop),
            Some(&Value::String(prop))
        );
    }

    proptest! {
        #[test]
        fn create_delete_sequence_preserves_alive_count(ops in proptest::collection::vec(any::<bool>(), 1..64)) {
            let shared = SharedGraph::new(GraphId::new(1));
            let mut txn = shared.begin_write();
            let mut expected_alive = BTreeSet::new();
            let mut created = Vec::new();
            {
                let mut mutator = txn.mutator();
                for delete_previous in ops {
                    let id = mutator.create_node(LabelSet::new(), PropertyMap::new());
                    expected_alive.insert(id);
                    created.push(id);
                    if delete_previous
                        && let Some(to_delete) = created.first().copied()
                        && expected_alive.remove(&to_delete)
                    {
                        mutator.delete_node(to_delete).unwrap();
                    }
                }
                prop_assert_eq!(mutator.read().node_count(), expected_alive.len());
                prop_assert_eq!(
                    mutator.read().meta.next_node_id,
                    1,
                    "working meta is updated at commit, allocator advances during mutation"
                );
            }
            let outcome = txn.commit().unwrap();
            prop_assert_eq!(shared.read().node_count(), expected_alive.len());
            prop_assert_eq!(outcome.next_node_id as usize, created.len() + 1);
        }
    }

    #[test]
    #[cfg(not(miri))]
    fn four_writer_stress_no_double_allocation() {
        let shared = Arc::new(SharedGraph::new(GraphId::new(1)));
        let nodes_per_thread = 64;
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let shared = Arc::clone(&shared);
                scope.spawn(move || {
                    let mut txn = shared.begin_write();
                    {
                        let mut mutator = txn.mutator();
                        for _ in 0..nodes_per_thread {
                            let _ = mutator.create_node(LabelSet::new(), PropertyMap::new());
                        }
                    }
                    txn.commit().unwrap();
                });
            }
        });
        let snapshot = shared.read();
        assert_eq!(snapshot.node_count(), 4 * nodes_per_thread);
        assert_eq!(
            snapshot.meta.next_node_id,
            (4 * nodes_per_thread + 1) as u64
        );
    }

    #[test]
    fn value_type_import_smoke_keeps_schema_deferred() {
        let value_type = ValueType::predefined(PredefinedValueType::String);
        assert_eq!(value_type.predefined, Some(PredefinedValueType::String));
    }
}
