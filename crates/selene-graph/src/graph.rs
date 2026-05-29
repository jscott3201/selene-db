//! Immutable graph snapshot and read accessors.

use std::borrow::Cow;
use std::ops::RangeBounds;
use std::sync::Arc;

use imbl::HashMap;
use roaring::RoaringBitmap;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use selene_core::{EdgeId, GraphId, IStr, LabelSet, NodeId, PropertyMap, Value};

use crate::adjacency::AdjacencyEntry;
use crate::composite_typed_index::CompositeTypedIndex;
use crate::graph_types::GraphTypeDef;
use crate::store::{EdgeStore, NodeStore, edge_row_index, node_row_index};
use crate::typed_index::{TypedIndex, TypedIndexKind};

/// Registered built-in property-index metadata.
#[derive(Clone, Debug)]
pub struct PropertyIndexEntry {
    /// Index data for the `(label, property)` registration.
    pub index: Arc<TypedIndex>,
    /// Optional explicit catalog name. `None` means the name is derived at render time.
    pub name: Option<IStr>,
}

impl PropertyIndexEntry {
    /// Construct an index entry from the built index and optional explicit name.
    #[must_use]
    pub fn new(index: TypedIndex, name: Option<IStr>) -> Self {
        Self {
            index: Arc::new(index),
            name,
        }
    }

    /// Return the registered index kind.
    #[must_use]
    pub fn kind(&self) -> TypedIndexKind {
        self.index.kind()
    }
}

/// Registered built-in composite-property index metadata.
#[derive(Clone, Debug)]
pub struct CompositePropertyIndexEntry {
    /// Index data for the `(label, properties...)` registration.
    pub index: Arc<CompositeTypedIndex>,
    /// Indexed properties in declaration order.
    pub declared_properties: SmallVec<[IStr; 4]>,
    /// Optional explicit catalog name. `None` means the name is derived at render time.
    pub name: Option<IStr>,
}

impl CompositePropertyIndexEntry {
    /// Construct a composite index entry.
    #[must_use]
    pub fn new(
        index: CompositeTypedIndex,
        declared_properties: SmallVec<[IStr; 4]>,
        name: Option<IStr>,
    ) -> Self {
        Self {
            index: Arc::new(index),
            declared_properties,
            name,
        }
    }

    /// Return the registered component kinds in declaration order.
    #[must_use]
    pub fn kinds(&self) -> SmallVec<[TypedIndexKind; 4]> {
        self.index.kinds().iter().copied().collect()
    }
}

/// Owned row returned when iterating composite property-index registrations.
pub type CompositePropertyIndexEntryRow = (
    IStr,
    SmallVec<[IStr; 4]>,
    SmallVec<[TypedIndexKind; 4]>,
    Option<IStr>,
);

/// Snapshot metadata.
#[derive(
    Clone,
    Debug,
    Deserialize,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct GraphMeta {
    /// Graph identifier.
    pub graph_id: GraphId,
    /// Published generation counter.
    pub generation: u64,
    /// Next node ID to allocate.
    pub next_node_id: u64,
    /// Next edge ID to allocate.
    pub next_edge_id: u64,
    /// Bound closed graph type. `None` means GG01/open graph.
    pub bound_type: Option<Arc<GraphTypeDef>>,
}

/// Immutable graph snapshot.
#[derive(Clone, Debug)]
pub struct SeleneGraph {
    /// Snapshot metadata.
    pub meta: GraphMeta,
    /// Node storage.
    pub node_store: NodeStore,
    /// Edge storage.
    pub edge_store: EdgeStore,
    /// Outgoing adjacency keyed by source node.
    pub adjacency_out: HashMap<NodeId, AdjacencyEntry>,
    /// Incoming adjacency keyed by target node.
    pub adjacency_in: HashMap<NodeId, AdjacencyEntry>,
    /// Bitmap of node rows carrying each label.
    pub idx_label: HashMap<IStr, RoaringBitmap>,
    /// Bitmap of edge rows carrying each edge label.
    pub idx_edge_label: HashMap<IStr, RoaringBitmap>,
    /// Per-`(label, property)` node value indexes. See spec 03 section 5.2.
    pub property_index: FxHashMap<(IStr, IStr), PropertyIndexEntry>,
    /// Per-`(label, properties...)` node composite value indexes.
    pub composite_property_index:
        FxHashMap<(IStr, SmallVec<[IStr; 4]>), CompositePropertyIndexEntry>,
}

impl SeleneGraph {
    /// Construct an empty graph snapshot.
    #[must_use]
    pub fn new(graph_id: GraphId) -> Self {
        Self {
            meta: GraphMeta {
                graph_id,
                generation: 0,
                next_node_id: 1,
                next_edge_id: 1,
                bound_type: None,
            },
            node_store: NodeStore::new(),
            edge_store: EdgeStore::new(),
            adjacency_out: HashMap::new(),
            adjacency_in: HashMap::new(),
            idx_label: HashMap::new(),
            idx_edge_label: HashMap::new(),
            property_index: FxHashMap::default(),
            composite_property_index: FxHashMap::default(),
        }
    }

    /// Return this graph snapshot's stable graph ID.
    #[must_use]
    pub const fn graph_id(&self) -> GraphId {
        self.meta.graph_id
    }

    /// Number of alive nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.node_store.alive.len() as usize
    }

    /// Bitmap of alive node *row indices*.
    ///
    /// Returned bitmap is row-indexed (matching `nodes_with_label`), not
    /// `NodeId`-indexed; consumers convert to `NodeId` via
    /// `selene_core::NodeId::new(row as u64 + 1)` (the inverse of
    /// `selene_graph::store::node_row_index`). Used by `selene-algorithms` to
    /// seed the "all alive nodes" baseline of a `GraphProjection`.
    #[must_use]
    pub fn live_nodes(&self) -> &RoaringBitmap {
        &self.node_store.alive
    }

    /// Number of alive edges.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edge_store.alive.len() as usize
    }

    /// Bitmap of alive edge *row indices*.
    ///
    /// The edge-side sibling of [`Self::live_nodes`]. The returned bitmap is
    /// row-indexed (matching `edges_with_label`), not `EdgeId`-indexed;
    /// consumers convert a row to an `EdgeId` via
    /// `selene_core::EdgeId::new(row as u64 + 1)` (the inverse of
    /// `selene_graph::store::edge_row_index`). Covers every alive edge
    /// regardless of label — used by the `DROP GRAPH` factory-reset (BRIEF-152)
    /// to enumerate every live edge, including untyped/arbitrary-label ones that
    /// a per-type truncate would miss.
    #[must_use]
    pub fn live_edges(&self) -> &RoaringBitmap {
        &self.edge_store.alive
    }

    /// Return true when `id` names an alive node.
    #[must_use]
    pub fn is_node_alive(&self, id: NodeId) -> bool {
        node_row_index(id).is_some_and(|row| {
            (row as usize) < self.node_store.len() && self.node_store.is_alive(row)
        })
    }

    /// Return true when `id` names an alive edge.
    #[must_use]
    pub fn is_edge_alive(&self, id: EdgeId) -> bool {
        edge_row_index(id).is_some_and(|row| {
            (row as usize) < self.edge_store.len() && self.edge_store.is_alive(row)
        })
    }

    /// Return node labels for an alive node.
    #[must_use]
    pub fn node_labels(&self, id: NodeId) -> Option<&LabelSet> {
        self.live_node_row(id)
            .and_then(|row| self.node_store.labels.get(row))
    }

    /// Return node properties for an alive node.
    #[must_use]
    pub fn node_properties(&self, id: NodeId) -> Option<&PropertyMap> {
        self.live_node_row(id)
            .and_then(|row| self.node_store.properties.get(row))
    }

    /// Return edge label for an alive edge.
    #[must_use]
    pub fn edge_label(&self, id: EdgeId) -> Option<&IStr> {
        self.live_edge_row(id)
            .and_then(|row| self.edge_store.label.get(row))
    }

    /// Return edge endpoints for an alive edge.
    #[must_use]
    pub fn edge_endpoints(&self, id: EdgeId) -> Option<(NodeId, NodeId)> {
        self.live_edge_row(id).and_then(|row| {
            Some((
                *self.edge_store.source.get(row)?,
                *self.edge_store.target.get(row)?,
            ))
        })
    }

    /// Return edge properties for an alive edge.
    #[must_use]
    pub fn edge_properties(&self, id: EdgeId) -> Option<&PropertyMap> {
        self.live_edge_row(id)
            .and_then(|row| self.edge_store.properties.get(row))
    }

    /// Return outgoing adjacency for `source`.
    #[must_use]
    pub fn outgoing_edges(&self, source: NodeId) -> Option<&AdjacencyEntry> {
        self.adjacency_out.get(&source)
    }

    /// Return incoming adjacency for `target`.
    #[must_use]
    pub fn incoming_edges(&self, target: NodeId) -> Option<&AdjacencyEntry> {
        self.adjacency_in.get(&target)
    }

    /// Return true when an alive node has at least one incident edge.
    #[must_use]
    pub fn node_has_incident_edges(&self, id: NodeId) -> bool {
        self.outgoing_edges(id)
            .is_some_and(|entry| !entry.is_empty())
            || self
                .incoming_edges(id)
                .is_some_and(|entry| !entry.is_empty())
    }

    /// Return the bitmap of node rows carrying `label`.
    #[must_use]
    pub fn nodes_with_label(&self, label: &IStr) -> Option<&RoaringBitmap> {
        self.idx_label.get(label)
    }

    /// Return the bitmap of edge rows carrying `label`.
    #[must_use]
    pub fn edges_with_label(&self, label: &IStr) -> Option<&RoaringBitmap> {
        self.idx_edge_label.get(label)
    }

    /// Number of distinct node labels currently indexed.
    #[must_use]
    pub fn label_count(&self) -> usize {
        self.idx_label.len()
    }

    /// Number of distinct edge labels currently indexed.
    #[must_use]
    pub fn edge_label_count(&self) -> usize {
        self.idx_edge_label.len()
    }

    /// Return a clone of the registered `(label, property)` index.
    #[must_use]
    pub fn property_index_for(&self, label: &IStr, property: &IStr) -> Option<Arc<TypedIndex>> {
        self.property_index
            .get(&(*label, *property))
            .map(|entry| Arc::clone(&entry.index))
    }

    /// Return a clone of the registered composite index.
    #[must_use]
    pub fn composite_property_index_for(
        &self,
        label: &IStr,
        properties: &[IStr],
    ) -> Option<Arc<CompositeTypedIndex>> {
        self.composite_property_index_entry_for(label, properties)
            .map(|entry| Arc::clone(&entry.index))
    }

    /// Return composite index metadata for a property set.
    #[must_use]
    pub fn composite_property_index_entry_for(
        &self,
        label: &IStr,
        properties: &[IStr],
    ) -> Option<&CompositePropertyIndexEntry> {
        let key = composite_property_key(properties);
        self.composite_property_index.get(&(*label, key))
    }

    /// Number of distinct `(label, property)` indexes currently registered.
    #[must_use]
    pub fn property_index_count(&self) -> usize {
        self.property_index.len()
    }

    /// Number of distinct `(label, properties...)` indexes currently registered.
    #[must_use]
    pub fn composite_property_index_count(&self) -> usize {
        self.composite_property_index.len()
    }

    /// Iterate built-in property indexes as owned `(label, property, kind)` tuples.
    ///
    /// This covers only SeleneGraph's built-in property indexes. Vector indexes
    /// are extension-provider state and remain available through
    /// `CALL vector.list_indexes()`.
    pub fn iter_property_indexes(&self) -> impl Iterator<Item = (IStr, IStr, TypedIndexKind)> + '_ {
        self.property_index
            .iter()
            .map(|((label, property), entry)| (*label, *property, entry.kind()))
    }

    /// Iterate built-in property indexes with optional explicit catalog names.
    pub fn iter_property_index_entries(
        &self,
    ) -> impl Iterator<Item = (IStr, IStr, TypedIndexKind, Option<IStr>)> + '_ {
        self.property_index
            .iter()
            .map(|((label, property), entry)| (*label, *property, entry.kind(), entry.name))
    }

    /// Iterate built-in composite property indexes with optional explicit catalog names.
    pub fn iter_composite_property_index_entries(
        &self,
    ) -> impl Iterator<Item = CompositePropertyIndexEntryRow> + '_ {
        self.composite_property_index
            .iter()
            .map(|((label, _), entry)| {
                (
                    *label,
                    entry.declared_properties.clone(),
                    entry.kinds(),
                    entry.name,
                )
            })
    }

    /// Return rows matching `value` under a registered property index.
    ///
    /// `None` means no index is registered for `(label, property)` or the
    /// supplied value cannot be used with that index kind. `Some(empty)` means
    /// the index exists but no row matches.
    ///
    /// For `STRING`-kind indexes, `Some(empty)` is also returned when
    /// `value` is a [`Value::ExternalString`] whose content is not in the
    /// global [`IStr`] pool — the read path MUST NOT admit, and an
    /// unpooled content cannot be keyed in the index (BRIEF-153). Other
    /// index kinds probed with `Value::ExternalString` return `None` so
    /// the caller drops to a linear scan; open-graph kind drift remains
    /// discoverable via cross-variant `value_compare`.
    #[must_use]
    pub fn nodes_with_property_eq(
        &self,
        label: &IStr,
        property: &IStr,
        value: &Value,
    ) -> Option<Cow<'_, RoaringBitmap>> {
        self.property_index
            .get(&(*label, *property))
            .and_then(|entry| entry.index.lookup_eq(value))
    }

    /// Return rows matching `range` under a registered property index.
    ///
    /// `None` means no index is registered or the supplied bounds do not match
    /// the index kind. `Some(empty)` means the index exists but the range
    /// matches no rows.
    #[must_use]
    pub fn nodes_with_property_range<R>(
        &self,
        label: &IStr,
        property: &IStr,
        range: R,
    ) -> Option<RoaringBitmap>
    where
        R: RangeBounds<Value>,
    {
        self.property_index
            .get(&(*label, *property))
            .and_then(|entry| entry.index.lookup_range(range))
    }

    /// Return rows whose string property key starts with `prefix`.
    ///
    /// `None` means no index is registered or the registered index is not a
    /// string index.
    #[must_use]
    pub fn nodes_with_property_prefix(
        &self,
        label: &IStr,
        property: &IStr,
        prefix: &str,
    ) -> Option<RoaringBitmap> {
        self.property_index
            .get(&(*label, *property))
            .and_then(|entry| entry.index.lookup_prefix(prefix))
    }

    fn live_node_row(&self, id: NodeId) -> Option<usize> {
        let row = node_row_index(id)?;
        ((row as usize) < self.node_store.len() && self.node_store.is_alive(row))
            .then_some(row as usize)
    }

    fn live_edge_row(&self, id: EdgeId) -> Option<usize> {
        let row = edge_row_index(id)?;
        ((row as usize) < self.edge_store.len() && self.edge_store.is_alive(row))
            .then_some(row as usize)
    }
}

pub(crate) fn composite_property_key(properties: &[IStr]) -> SmallVec<[IStr; 4]> {
    let mut key: SmallVec<[IStr; 4]> = properties.iter().copied().collect();
    key.sort();
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use selene_core::intern;

    #[test]
    fn new_graph_is_empty() {
        let graph = SeleneGraph::new(GraphId::new(1));
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(graph.label_count(), 0);
        assert_eq!(graph.edge_label_count(), 0);
        assert_eq!(graph.property_index_count(), 0);
        assert_eq!(graph.composite_property_index_count(), 0);
        assert!(graph.idx_label.is_empty());
        assert!(graph.idx_edge_label.is_empty());
        assert!(graph.property_index.is_empty());
        assert!(graph.composite_property_index.is_empty());
        assert_eq!(graph.meta.generation, 0);
        assert_eq!(graph.meta.next_node_id, 1);
        assert_eq!(graph.meta.next_edge_id, 1);
    }

    #[test]
    fn read_accessors_return_none_for_unknown_ids() {
        let graph = SeleneGraph::new(GraphId::new(1));
        assert_eq!(graph.node_labels(NodeId::new(1)), None);
        assert_eq!(graph.edge_label(EdgeId::new(1)), None);
        assert_eq!(
            graph.nodes_with_label(&intern("graph.missing").unwrap()),
            None
        );
        assert!(!graph.is_node_alive(NodeId::TOMBSTONE));
    }

    #[test]
    fn node_labels_returns_some_for_alive_node() {
        let mut graph = SeleneGraph::new(GraphId::new(1));
        let label = intern("graph.node").unwrap();
        graph.node_store.labels.push(LabelSet::single(label));
        graph.node_store.properties.push(PropertyMap::new());
        graph.node_store.alive.insert(0);
        assert_eq!(
            graph
                .node_labels(NodeId::new(1))
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![label]
        );
    }

    #[test]
    fn label_count_reports_distinct_labels_only() {
        let mut graph = SeleneGraph::new(GraphId::new(1));
        let label = intern("graph.same").unwrap();
        let mut bitmap = RoaringBitmap::new();
        bitmap.insert(0);
        bitmap.insert(1);
        graph.idx_label.insert(label, bitmap);
        assert_eq!(graph.label_count(), 1);
        assert!(graph.nodes_with_label(&label).unwrap().contains(0));
        assert!(graph.nodes_with_label(&label).unwrap().contains(1));
    }
}
