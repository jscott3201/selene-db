//! Typed mutation funnel per spec 03 section 4.3.

mod catalog;
mod composite_property_index;
mod property_index;
mod remove;

use std::collections::BTreeSet;
use std::sync::Arc;

use roaring::RoaringBitmap;
use selene_core::{
    Change, EdgeId, GraphId, IStr, LabelDiff, LabelSet, NodeId, Origin, PropertyDiff, PropertyMap,
    SchemaChange,
};

use crate::adjacency::{AdjacencyEdge, AdjacencyEntry};
use crate::error::{GraphError, GraphResult};
use crate::graph_types::{GraphTypeDef, PropertyTypeDef};
use crate::index_provider::{IndexProvider, ProviderTag};
use crate::store::{edge_row_index, node_row_index};
use crate::type_validator::{EntityId, TypeViolation};
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
    pub fn create_node(&mut self, labels: LabelSet, mut props: PropertyMap) -> GraphResult<NodeId> {
        fill_node_defaults(self.txn.read(), &labels, &mut props)?;
        let id = self.txn.allocator.allocate_node();
        let row = node_row_index(id).ok_or_else(|| GraphError::IdOverflow {
            kind: "node",
            raw: id.get(),
            max: u32::MAX as u64 + 1,
        })? as usize;
        {
            let graph = self.txn.guard_mut();
            ensure_node_rows(graph, row);
            // BRIEF-153 fix-cycle C2: run property-index admission BEFORE
            // mutating row state so a cap-exhaustion error rolls back
            // cleanly with no half-written row. Index updates only touch
            // their own maps; node_store stays untouched if any admission
            // fails. The txn boundary publishes everything atomically on
            // commit, so a partial-index publication is impossible.
            crate::property_index::apply_node_create(
                &mut graph.property_index,
                &labels,
                &props,
                row as u32,
            )?;
            crate::composite_property_index::apply_node_create(
                &mut graph.composite_property_index,
                &labels,
                &props,
                row as u32,
            )?;
            if row == graph.node_store.len() {
                graph.node_store.labels.push(labels.clone());
                graph.node_store.properties.push(props.clone());
            } else {
                graph.node_store.labels.set(row, labels.clone());
                graph.node_store.properties.set(row, props.clone());
            }
            graph.node_store.alive.insert(row as u32);
            insert_node_labels(&mut graph.idx_label, row as u32, &labels);
        }
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
        mut props: PropertyMap,
    ) -> GraphResult<EdgeId> {
        self.require_live_node(source)?;
        self.require_live_node(target)?;
        fill_edge_defaults(self.txn.read(), label, source, target, &mut props)?;
        let id = self.txn.allocator.allocate_edge();
        let row = edge_row_index(id).ok_or_else(|| GraphError::IdOverflow {
            kind: "edge",
            raw: id.get(),
            max: u32::MAX as u64 + 1,
        })? as usize;
        {
            let graph = self.txn.guard_mut();
            ensure_edge_rows(graph, row)?;
            if row == graph.edge_store.len() {
                graph.edge_store.label.push(label);
                graph.edge_store.source.push(source);
                graph.edge_store.target.push(target);
                graph.edge_store.properties.push(props.clone());
            } else {
                graph.edge_store.label.set(row, label);
                graph.edge_store.source.set(row, source);
                graph.edge_store.target.set(row, target);
                graph.edge_store.properties.set(row, props.clone());
            }
            graph.edge_store.alive.insert(row as u32);
            insert_index_row(&mut graph.idx_edge_label, label, row as u32);

            graph
                .adjacency_out
                .entry(source)
                .or_default()
                .add(AdjacencyEdge {
                    label,
                    neighbor: target,
                    edge_id: id,
                });
            graph
                .adjacency_in
                .entry(target)
                .or_default()
                .add(AdjacencyEdge {
                    label,
                    neighbor: source,
                    edge_id: id,
                });
        }
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
        let old_labels = self
            .txn
            .read()
            .node_store
            .labels
            .get(row)
            .cloned()
            .unwrap_or_default();
        let mut labels = old_labels.clone();
        for label in labels_diff.added.iter().copied() {
            labels.insert(label);
        }
        for label in labels_diff.removed.iter() {
            labels.remove(label);
        }
        reject_immutable_node_update(self.txn.read(), id, &old_labels, &props_diff)?;
        reject_immutable_node_update(self.txn.read(), id, &labels, &props_diff)?;

        // Apply the property diff up front; if it errors we leave the working
        // graph (including idx_label) untouched so the transaction can still
        // be safely rolled back or aborted without leaking inconsistent state.
        let old_props = self
            .txn
            .read()
            .node_store
            .properties
            .get(row)
            .cloned()
            .unwrap_or_default();
        let mut props = old_props.clone();
        apply_property_diff(&mut props, &props_diff)?;

        // BRIEF-153 fix-cycle C2: run property-index admission BEFORE
        // mutating row state so a cap-exhaustion error rolls back cleanly
        // with no half-written row.
        let new_labels = labels.clone();
        let new_props = props.clone();
        {
            let graph = self.txn.guard_mut();
            crate::property_index::apply_node_update(
                &mut graph.property_index,
                &old_labels,
                &old_props,
                &new_labels,
                &new_props,
                row as u32,
            )?;
            crate::composite_property_index::apply_node_update(
                &mut graph.composite_property_index,
                &old_labels,
                &old_props,
                &new_labels,
                &new_props,
                row as u32,
            )?;
            graph.node_store.labels.set(row, labels);
            graph.node_store.properties.set(row, props);
            for label in labels_diff.added.iter().copied() {
                insert_index_row(&mut graph.idx_label, label, row as u32);
            }
            for label in labels_diff.removed.iter() {
                remove_index_row(&mut graph.idx_label, label, row as u32);
            }
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
        reject_immutable_edge_update(self.txn.read(), id, &props_diff)?;
        let mut props = self
            .txn
            .read()
            .edge_store
            .properties
            .get(row)
            .cloned()
            .unwrap_or_default();
        apply_property_diff(&mut props, &props_diff)?;
        self.txn.guard_mut().edge_store.properties.set(row, props);
        self.txn.changes.push(Change::EdgeUpdated {
            id,
            properties_diff: props_diff,
        });
        Ok(())
    }

    /// Delete an alive node and cascade delete incident edges.
    pub fn delete_node(&mut self, id: NodeId) -> GraphResult<()> {
        let row = self.require_live_node(id)?;
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
            graph.node_store.alive.remove(row as u32);
        }
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
    /// This is a pass-through accumulator. Catalog graph mutation and
    /// closed-graph validation are handled by higher-level validation layers.
    pub fn schema_change(&mut self, graph: GraphId, change: SchemaChange) {
        self.txn
            .changes
            .push(Change::SchemaChanged { graph, change });
    }

    /// Append an opaque extension-provider event.
    ///
    /// Replay is owned by registered index providers.
    pub fn extension_event(&mut self, provider: IStr, payload: Arc<[u8]>) {
        self.txn
            .changes
            .push(Change::IndexExtensionEvent { provider, payload });
    }

    /// Look up a registered index provider through the held write transaction.
    #[must_use]
    pub fn index_provider_by_tag(&self, tag: ProviderTag) -> Option<Arc<dyn IndexProvider>> {
        self.txn
            .providers
            .iter()
            .find(|provider| provider.provider_tag() == tag)
            .map(Arc::clone)
    }

    /// Borrow the transaction-local working graph.
    #[must_use]
    pub fn read(&self) -> &crate::SeleneGraph {
        self.txn.read()
    }

    fn delete_edge_inner(&mut self, id: EdgeId, record_change: bool) -> GraphResult<()> {
        let row = self.require_live_edge(id)?;
        let graph = self.txn.read();
        let label = *graph
            .edge_store
            .label
            .get(row)
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
        {
            let graph = self.txn.guard_mut();
            graph.edge_store.alive.remove(row as u32);
            remove_index_row(&mut graph.idx_edge_label, &label, row as u32);
            if let Some(mut entry) = graph.adjacency_out.get(&source).cloned() {
                entry.remove(id);
                update_or_remove_entry(&mut graph.adjacency_out, source, entry);
            }
            if let Some(mut entry) = graph.adjacency_in.get(&target).cloned() {
                entry.remove(id);
                update_or_remove_entry(&mut graph.adjacency_in, target, entry);
            }
        }
        if record_change {
            self.txn.changes.push(Change::EdgeDeleted { id });
        }
        Ok(())
    }

    fn require_live_node(&self, id: NodeId) -> GraphResult<usize> {
        let row = node_row_index(id).ok_or(GraphError::NodeNotFound { id })?;
        let graph = self.txn.read();
        if row as usize >= graph.node_store.len() {
            return Err(GraphError::NodeNotFound { id });
        }
        if !graph.node_store.is_alive(row) {
            return Err(GraphError::NodeNotAlive { id });
        }
        Ok(row as usize)
    }

    fn require_live_edge(&self, id: EdgeId) -> GraphResult<usize> {
        let row = edge_row_index(id).ok_or(GraphError::EdgeNotFound { id })?;
        let graph = self.txn.read();
        if row as usize >= graph.edge_store.len() {
            return Err(GraphError::EdgeNotFound { id });
        }
        if !graph.edge_store.is_alive(row) {
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

fn fill_node_defaults(
    graph: &crate::SeleneGraph,
    labels: &LabelSet,
    props: &mut PropertyMap,
) -> GraphResult<()> {
    let Some(graph_type) = graph.meta.bound_type.as_deref() else {
        return Ok(());
    };
    let Some(node_type) = graph_type.find_node_type(labels) else {
        return Ok(());
    };
    fill_property_defaults(&node_type.properties, props)
}

fn fill_edge_defaults(
    graph: &crate::SeleneGraph,
    label: IStr,
    source: NodeId,
    target: NodeId,
    props: &mut PropertyMap,
) -> GraphResult<()> {
    let Some(graph_type) = graph.meta.bound_type.as_deref() else {
        return Ok(());
    };
    let Some(source_type) = node_type_index_for_node(graph, graph_type, source) else {
        return Ok(());
    };
    let Some(target_type) = node_type_index_for_node(graph, graph_type, target) else {
        return Ok(());
    };
    let Some(edge_type) = graph_type.find_edge_type(label, source_type, target_type) else {
        return Ok(());
    };
    fill_property_defaults(&edge_type.properties, props)
}

fn fill_property_defaults(
    declarations: &[PropertyTypeDef],
    props: &mut PropertyMap,
) -> GraphResult<()> {
    for declaration in declarations {
        if props.contains_key(&declaration.name) {
            continue;
        }
        if let Some(default) = &declaration.default {
            props.set(declaration.name, default.to_value())?;
        }
    }
    Ok(())
}

fn reject_immutable_node_update(
    graph: &crate::SeleneGraph,
    id: NodeId,
    labels: &LabelSet,
    diff: &PropertyDiff,
) -> GraphResult<()> {
    let Some(graph_type) = graph.meta.bound_type.as_deref() else {
        return Ok(());
    };
    let Some(node_type) = graph_type.find_node_type(labels) else {
        return Ok(());
    };
    reject_immutable_property_update(
        EntityId::Node(id),
        node_type.name,
        &node_type.properties,
        diff,
    )
}

fn reject_immutable_edge_update(
    graph: &crate::SeleneGraph,
    id: EdgeId,
    diff: &PropertyDiff,
) -> GraphResult<()> {
    let Some(graph_type) = graph.meta.bound_type.as_deref() else {
        return Ok(());
    };
    let Some(label) = graph.edge_label(id).copied() else {
        return Ok(());
    };
    let Some((source, target)) = graph.edge_endpoints(id) else {
        return Ok(());
    };
    let Some(source_type) = node_type_index_for_node(graph, graph_type, source) else {
        return Ok(());
    };
    let Some(target_type) = node_type_index_for_node(graph, graph_type, target) else {
        return Ok(());
    };
    let Some(edge_type) = graph_type.find_edge_type(label, source_type, target_type) else {
        return Ok(());
    };
    reject_immutable_property_update(
        EntityId::Edge(id),
        edge_type.name,
        &edge_type.properties,
        diff,
    )
}

fn node_type_index_for_node(
    graph: &crate::SeleneGraph,
    graph_type: &GraphTypeDef,
    node: NodeId,
) -> Option<u32> {
    let labels = graph.node_labels(node)?;
    graph_type.find_node_type_index(labels)
}

fn reject_immutable_property_update(
    entity_id: EntityId,
    declared_in: IStr,
    declarations: &[PropertyTypeDef],
    diff: &PropertyDiff,
) -> GraphResult<()> {
    for (key, _) in &diff.set {
        reject_if_immutable(entity_id, declared_in, declarations, *key)?;
    }
    for key in &diff.removed {
        reject_if_immutable(entity_id, declared_in, declarations, *key)?;
    }
    Ok(())
}

fn reject_if_immutable(
    entity_id: EntityId,
    declared_in: IStr,
    declarations: &[PropertyTypeDef],
    property: IStr,
) -> GraphResult<()> {
    if declarations
        .iter()
        .any(|declaration| declaration.name == property && declaration.immutable)
    {
        return Err(GraphError::TypeViolation(
            TypeViolation::ImmutablePropertyUpdate {
                entity_id,
                property,
                declared_in,
            },
        ));
    }
    Ok(())
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
