//! Snapshot-scoped physical-row candidate sets.

use std::fmt;
use std::marker::PhantomData;
use std::ops::RangeBounds;
use std::sync::Arc;

use roaring::RoaringBitmap;
use selene_core::{DbString, EdgeId, GraphId, NodeId, Value};

use crate::store::{EdgeRow, NodeRow};
use crate::{CompositeKey, SeleneGraph};

mod sealed {
    pub trait Sealed {}
}

/// Sealed marker implemented only by [`Node`] and [`Edge`].
pub trait CandidateKind: sealed::Sealed + Send + Sync + 'static {}

/// Marker for node candidate sets.
pub enum Node {}

impl sealed::Sealed for Node {}
impl CandidateKind for Node {}

/// Marker for edge candidate sets.
pub enum Edge {}

impl sealed::Sealed for Edge {}
impl CandidateKind for Edge {}

#[derive(Clone)]
pub(crate) struct LayoutToken(Arc<LayoutIdentity>);

#[derive(Debug)]
struct LayoutIdentity;

impl LayoutToken {
    pub(crate) fn new() -> Self {
        Self(Arc::new(LayoutIdentity))
    }

    fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for LayoutToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LayoutToken")
    }
}

/// Scope mismatch returned by candidate validation or algebra.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum CandidateSetError {
    /// Candidate and graph identifiers differ.
    #[error("candidate graph {expected} does not match graph {actual}")]
    GraphMismatch {
        /// Graph identifier retained by the candidate set.
        expected: GraphId,
        /// Graph identifier supplied for validation.
        actual: GraphId,
    },
    /// Candidate and graph generations differ.
    #[error("candidate generation {expected} does not match graph {graph_id} generation {actual}")]
    GenerationMismatch {
        /// Graph identifier shared by both scopes.
        graph_id: GraphId,
        /// Generation retained by the candidate set.
        expected: u64,
        /// Generation supplied for validation.
        actual: u64,
    },
    /// Candidate and graph have independent physical row layouts.
    #[error(
        "candidate layout for graph {expected_graph_id} generation {expected_generation} does not match supplied graph {actual_graph_id} generation {actual_generation}"
    )]
    LayoutMismatch {
        /// Graph identifier retained by the candidate set.
        expected_graph_id: GraphId,
        /// Generation retained by the candidate set.
        expected_generation: u64,
        /// Graph identifier supplied for validation.
        actual_graph_id: GraphId,
        /// Generation supplied for validation.
        actual_generation: u64,
    },
}

/// A graph-, generation-, layout-, and entity-kind-bound candidate set.
///
/// Candidate sets are ephemeral and intentionally do not implement persistence
/// codecs. Public access yields stable IDs only. Physical rows and the layout
/// token remain private to trusted graph-owned producers and consumers.
///
/// Node and edge sets cannot be mixed:
///
/// ```compile_fail
/// use selene_graph::{CandidateSet, Edge, Node};
/// fn combine(nodes: &CandidateSet<Node>, edges: &CandidateSet<Edge>) {
///     let _ = nodes.union(edges);
/// }
/// ```
///
/// Physical rows are not publicly iterable:
///
/// ```compile_fail
/// use selene_graph::{CandidateSet, Node};
/// fn rows(candidates: &CandidateSet<Node>) { let _ = candidates.rows(); }
/// ```
#[derive(Clone)]
pub struct CandidateSet<K: CandidateKind> {
    graph_id: GraphId,
    generation: u64,
    layout: LayoutToken,
    rows: Arc<RoaringBitmap>,
    kind: PhantomData<K>,
}

impl<K: CandidateKind> CandidateSet<K> {
    fn from_bitmap(graph: &SeleneGraph, rows: RoaringBitmap) -> Self {
        Self::from_bitmap_arc(graph, Arc::new(rows))
    }

    fn from_bitmap_arc(graph: &SeleneGraph, rows: Arc<RoaringBitmap>) -> Self {
        Self {
            graph_id: graph.meta.graph_id,
            generation: graph.meta.generation,
            layout: graph.layout.clone(),
            rows,
            kind: PhantomData,
        }
    }

    /// Return the number of candidate entities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len() as usize
    }

    /// Return true when the set has no candidates.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Validate that this set belongs to the exact supplied graph snapshot scope.
    pub fn validate_scope(&self, graph: &SeleneGraph) -> Result<(), CandidateSetError> {
        validate_scope(
            self.graph_id,
            self.generation,
            &self.layout,
            graph.meta.graph_id,
            graph.meta.generation,
            &graph.layout,
        )
    }

    /// Return the union of two identically scoped candidate sets.
    pub fn union(&self, other: &Self) -> Result<Self, CandidateSetError> {
        self.validate_other(other)?;
        let mut rows = self.rows.as_ref().clone();
        rows |= other.rows.as_ref();
        Ok(self.with_rows(rows))
    }

    /// Return the intersection of two identically scoped candidate sets.
    pub fn intersection(&self, other: &Self) -> Result<Self, CandidateSetError> {
        self.validate_other(other)?;
        let mut rows = self.rows.as_ref().clone();
        rows &= other.rows.as_ref();
        Ok(self.with_rows(rows))
    }

    /// Return candidates in this set but not in `other`.
    pub fn difference(&self, other: &Self) -> Result<Self, CandidateSetError> {
        self.validate_other(other)?;
        let mut rows = self.rows.as_ref().clone();
        rows -= other.rows.as_ref();
        Ok(self.with_rows(rows))
    }

    pub(crate) fn raw_rows(&self) -> impl Iterator<Item = u32> + '_ {
        self.rows.iter()
    }

    pub(crate) fn bitmap(&self) -> &RoaringBitmap {
        &self.rows
    }

    fn validate_other(&self, other: &Self) -> Result<(), CandidateSetError> {
        validate_scope(
            self.graph_id,
            self.generation,
            &self.layout,
            other.graph_id,
            other.generation,
            &other.layout,
        )
    }

    fn with_rows(&self, rows: RoaringBitmap) -> Self {
        Self {
            graph_id: self.graph_id,
            generation: self.generation,
            layout: self.layout.clone(),
            rows: Arc::new(rows),
            kind: PhantomData,
        }
    }
}

impl CandidateSet<Node> {
    pub(crate) fn from_node_rows(graph: &SeleneGraph, rows: RoaringBitmap) -> Self {
        Self::from_bitmap(graph, rows)
    }

    pub(crate) fn from_node_rows_arc(graph: &SeleneGraph, rows: Arc<RoaringBitmap>) -> Self {
        Self::from_bitmap_arc(graph, rows)
    }

    pub(crate) fn node_rows(&self) -> impl Iterator<Item = NodeRow> + '_ {
        self.raw_rows().map(NodeRow::new)
    }

    pub(crate) fn try_from_ids(
        graph: &SeleneGraph,
        ids: impl IntoIterator<Item = NodeId>,
    ) -> Option<Self> {
        let mut rows = RoaringBitmap::new();
        for id in ids {
            let row = graph.node_row_for_id(id)?;
            if !graph.node_store.is_row_alive(row) {
                return None;
            }
            rows.insert(row.get());
        }
        Some(Self::from_node_rows(graph, rows))
    }

    /// Iterate stable node IDs in deterministic physical-row order.
    pub fn iter_ids<'a>(
        &'a self,
        graph: &'a SeleneGraph,
    ) -> Result<impl Iterator<Item = NodeId> + 'a, CandidateSetError> {
        self.validate_scope(graph)?;
        Ok(self.node_rows().map(|row| {
            graph
                .node_id_for_node_row(row)
                .expect("trusted node candidate row has a stable ID")
        }))
    }

    /// Return whether this exact snapshot-scoped set contains `id`.
    pub fn contains_id(&self, graph: &SeleneGraph, id: NodeId) -> Result<bool, CandidateSetError> {
        self.validate_scope(graph)?;
        Ok(graph
            .node_row_for_id(id)
            .is_some_and(|row| self.rows.contains(row.get())))
    }
}

impl CandidateSet<Edge> {
    pub(crate) fn from_edge_rows(graph: &SeleneGraph, rows: RoaringBitmap) -> Self {
        Self::from_bitmap(graph, rows)
    }

    pub(crate) fn from_edge_rows_arc(graph: &SeleneGraph, rows: Arc<RoaringBitmap>) -> Self {
        Self::from_bitmap_arc(graph, rows)
    }

    pub(crate) fn edge_rows(&self) -> impl Iterator<Item = EdgeRow> + '_ {
        self.raw_rows().map(EdgeRow::new)
    }

    /// Iterate stable edge IDs in deterministic physical-row order.
    pub fn iter_ids<'a>(
        &'a self,
        graph: &'a SeleneGraph,
    ) -> Result<impl Iterator<Item = EdgeId> + 'a, CandidateSetError> {
        self.validate_scope(graph)?;
        Ok(self.edge_rows().map(|row| {
            graph
                .edge_id_for_edge_row(row)
                .expect("trusted edge candidate row has a stable ID")
        }))
    }

    /// Return whether this exact snapshot-scoped set contains `id`.
    pub fn contains_id(&self, graph: &SeleneGraph, id: EdgeId) -> Result<bool, CandidateSetError> {
        self.validate_scope(graph)?;
        Ok(graph
            .edge_row_for_id(id)
            .is_some_and(|row| self.rows.contains(row.get())))
    }
}

impl<K: CandidateKind> fmt::Debug for CandidateSet<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateSet")
            .field("graph_id", &self.graph_id)
            .field("generation", &self.generation)
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl SeleneGraph {
    /// Return all live nodes as a typed candidate set.
    #[must_use]
    pub fn live_node_candidates(&self) -> CandidateSet<Node> {
        CandidateSet::from_node_rows_arc(self, Arc::clone(&self.node_store.alive))
    }

    /// Return all live edges as a typed candidate set.
    #[must_use]
    pub fn live_edge_candidates(&self) -> CandidateSet<Edge> {
        CandidateSet::from_edge_rows_arc(self, Arc::clone(&self.edge_store.alive))
    }

    /// Return node candidates carrying `label`.
    #[must_use]
    pub fn node_label_candidates(&self, label: &DbString) -> CandidateSet<Node> {
        CandidateSet::from_node_rows(self, self.idx_label.get(label).cloned().unwrap_or_default())
    }

    /// Return edge candidates carrying `label`.
    #[must_use]
    pub fn edge_label_candidates(&self, label: &DbString) -> CandidateSet<Edge> {
        CandidateSet::from_edge_rows(
            self,
            self.idx_edge_label.get(label).cloned().unwrap_or_default(),
        )
    }

    /// Probe a node property index for equality candidates.
    #[must_use]
    pub fn node_property_eq_candidates(
        &self,
        label: &DbString,
        property: &DbString,
        value: &Value,
    ) -> Option<CandidateSet<Node>> {
        self.nodes_with_property_eq(label, property, value)
            .map(|rows| CandidateSet::from_node_rows(self, rows.into_owned()))
    }

    /// Probe a node property index for candidates matching any supplied value.
    #[must_use]
    pub fn node_property_any_candidates(
        &self,
        label: &DbString,
        property: &DbString,
        values: &[Value],
    ) -> Option<CandidateSet<Node>> {
        self.nodes_with_property_any(label, property, values)
            .map(|rows| CandidateSet::from_node_rows(self, rows))
    }

    /// Probe an edge property index for equality candidates.
    #[must_use]
    pub fn edge_property_eq_candidates(
        &self,
        label: &DbString,
        property: &DbString,
        value: &Value,
    ) -> Option<CandidateSet<Edge>> {
        self.edges_with_property_eq(label, property, value)
            .map(|rows| CandidateSet::from_edge_rows(self, rows.into_owned()))
    }

    /// Probe an edge property index for candidates matching any supplied value.
    #[must_use]
    pub fn edge_property_any_candidates(
        &self,
        label: &DbString,
        property: &DbString,
        values: &[Value],
    ) -> Option<CandidateSet<Edge>> {
        self.edges_with_property_any(label, property, values)
            .map(|rows| CandidateSet::from_edge_rows(self, rows))
    }

    /// Probe a node property index for range candidates.
    #[must_use]
    pub fn node_property_range_candidates<R>(
        &self,
        label: &DbString,
        property: &DbString,
        range: R,
    ) -> Option<CandidateSet<Node>>
    where
        R: RangeBounds<Value>,
    {
        self.nodes_with_property_range(label, property, range)
            .map(|rows| CandidateSet::from_node_rows(self, rows))
    }

    /// Probe an edge property index for range candidates.
    #[must_use]
    pub fn edge_property_range_candidates<R>(
        &self,
        label: &DbString,
        property: &DbString,
        range: R,
    ) -> Option<CandidateSet<Edge>>
    where
        R: RangeBounds<Value>,
    {
        self.edges_with_property_range(label, property, range)
            .map(|rows| CandidateSet::from_edge_rows(self, rows))
    }

    /// Probe a node composite-property index for an exact key.
    #[must_use]
    pub fn node_composite_property_candidates(
        &self,
        label: &DbString,
        properties: &[DbString],
        key: &CompositeKey,
    ) -> Option<CandidateSet<Node>> {
        self.composite_property_index_entry_for(label, properties)?
            .probe_arc()?
            .lookup_key(key)
            .cloned()
            .map(|rows| CandidateSet::from_node_rows(self, rows))
    }

    /// Probe a node string-property index for prefix candidates.
    #[must_use]
    pub fn node_property_prefix_candidates(
        &self,
        label: &DbString,
        property: &DbString,
        prefix: &str,
    ) -> Option<CandidateSet<Node>> {
        self.nodes_with_property_prefix(label, property, prefix)
            .map(|rows| CandidateSet::from_node_rows(self, rows))
    }

    /// Probe an edge string-property index for prefix candidates.
    #[must_use]
    pub fn edge_property_prefix_candidates(
        &self,
        label: &DbString,
        property: &DbString,
        prefix: &str,
    ) -> Option<CandidateSet<Edge>> {
        self.edge_property_index
            .get(&(label.clone(), property.clone()))
            .and_then(|entry| entry.lookup_prefix(prefix))
            .map(|rows| CandidateSet::from_edge_rows(self, rows))
    }
}

fn validate_scope(
    expected_graph: GraphId,
    expected_generation: u64,
    expected_layout: &LayoutToken,
    actual_graph: GraphId,
    actual_generation: u64,
    actual_layout: &LayoutToken,
) -> Result<(), CandidateSetError> {
    if expected_graph != actual_graph {
        return Err(CandidateSetError::GraphMismatch {
            expected: expected_graph,
            actual: actual_graph,
        });
    }
    if expected_generation != actual_generation {
        return Err(CandidateSetError::GenerationMismatch {
            graph_id: expected_graph,
            expected: expected_generation,
            actual: actual_generation,
        });
    }
    if !expected_layout.same_as(actual_layout) {
        return Err(CandidateSetError::LayoutMismatch {
            expected_graph_id: expected_graph,
            expected_generation,
            actual_graph_id: actual_graph,
            actual_generation,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
