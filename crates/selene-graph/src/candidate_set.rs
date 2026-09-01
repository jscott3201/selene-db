//! Ephemeral typed candidate sets bound to one immutable graph layout.

use std::marker::PhantomData;
use std::sync::Arc;

use selene_core::{EdgeId, GraphId, NodeId};

use crate::error::{CandidateSetError, CandidateSetResult, GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::store::{EdgeRow, NodeRow};

mod sealed {
    pub trait Sealed {}
}

/// A sealed element kind supported by [`CandidateSet`].
///
/// The graph crate implements this trait only for [`Node`] and [`Edge`], so
/// candidate kinds cannot be extended with an untrusted physical-row meaning.
pub trait CandidateKind: sealed::Sealed + 'static {
    /// Stable public identifier observed when iterating this candidate kind.
    type Id: Copy + std::fmt::Debug + Eq + Ord;
}

/// Marker kind for node candidate sets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Node {}

impl sealed::Sealed for Node {}

impl CandidateKind for Node {
    type Id = NodeId;
}

/// Marker kind for edge candidate sets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edge {}

impl sealed::Sealed for Edge {}

impl CandidateKind for Edge {
    type Id = EdgeId;
}

#[derive(Debug)]
pub(crate) struct LayoutToken;

#[derive(Clone, Debug)]
pub(crate) struct SnapshotLayout(Arc<LayoutToken>);

impl SnapshotLayout {
    pub(crate) fn new() -> Self {
        Self(Arc::new(LayoutToken))
    }

    fn retained(&self) -> Arc<LayoutToken> {
        Arc::clone(&self.0)
    }

    fn matches(&self, token: &Arc<LayoutToken>) -> bool {
        Arc::ptr_eq(&self.0, token)
    }

    #[cfg(test)]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

struct CandidateRow<K> {
    raw: u32,
    kind: PhantomData<fn() -> K>,
}

impl<K> CandidateRow<K> {
    const fn new(raw: u32) -> Self {
        Self {
            raw,
            kind: PhantomData,
        }
    }
}

impl<K> Clone for CandidateRow<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for CandidateRow<K> {}

impl<K> std::fmt::Debug for CandidateRow<K> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<private physical row>")
    }
}

/// An immutable, non-serialized set of candidates from one graph snapshot.
///
/// A set is bound to the producing lower graph identity, immutable generation,
/// and private snapshot-layout allocation. Graph-owned algebra rejects any
/// mismatch rather than translating physical rows. Public iteration is
/// deterministic in ascending stable-ID order and never exposes physical rows.
///
/// Node and edge sets cannot be mixed:
///
/// ```compile_fail,E0308
/// use selene_core::GraphId;
/// use selene_graph::SeleneGraph;
///
/// let graph = SeleneGraph::new(GraphId::new(1));
/// let nodes = graph.live_node_candidates().unwrap();
/// let edges = graph.live_edge_candidates().unwrap();
/// let _ = graph.union_candidates(&nodes, &edges);
/// ```
#[must_use]
pub struct CandidateSet<K: CandidateKind> {
    graph_id: GraphId,
    generation: u64,
    layout: Arc<LayoutToken>,
    entries: Arc<[(K::Id, CandidateRow<K>)]>,
}

impl<K: CandidateKind> Clone for CandidateSet<K> {
    fn clone(&self) -> Self {
        Self {
            graph_id: self.graph_id,
            generation: self.generation,
            layout: Arc::clone(&self.layout),
            entries: Arc::clone(&self.entries),
        }
    }
}

impl<K: CandidateKind> std::fmt::Debug for CandidateSet<K> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CandidateSet")
            .field("ids", &self.iter().collect::<Vec<_>>())
            .finish()
    }
}

impl<K: CandidateKind> CandidateSet<K> {
    /// Return the number of stable element IDs in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return true when the set contains no stable element IDs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return true when the set contains `id`.
    #[must_use]
    pub fn contains(&self, id: K::Id) -> bool {
        self.entries
            .binary_search_by_key(&id, |(candidate, _)| *candidate)
            .is_ok()
    }

    /// Iterate stable IDs in deterministic ascending order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = K::Id> + '_ {
        self.entries.iter().map(|(id, _)| *id)
    }

    fn from_sorted_entries(
        graph_id: GraphId,
        generation: u64,
        layout: &SnapshotLayout,
        entries: Vec<(K::Id, CandidateRow<K>)>,
    ) -> Self {
        debug_assert!(entries.windows(2).all(|pair| pair[0].0 < pair[1].0));
        Self {
            graph_id,
            generation,
            layout: layout.retained(),
            entries: entries.into(),
        }
    }

    fn validate_for(&self, graph: &SeleneGraph) -> CandidateSetResult<()> {
        if self.graph_id != graph.meta.graph_id {
            return Err(CandidateSetError::GraphMismatch {
                expected: graph.meta.graph_id,
                actual: self.graph_id,
            });
        }
        if self.generation != graph.meta.generation {
            return Err(CandidateSetError::GenerationMismatch {
                expected: graph.meta.generation,
                actual: self.generation,
            });
        }
        if !graph.layout.matches(&self.layout) {
            return Err(CandidateSetError::LayoutMismatch);
        }
        Ok(())
    }

    fn union(&self, other: &Self) -> Self {
        let mut entries = Vec::with_capacity(self.len().saturating_add(other.len()));
        merge_entries(&self.entries, &other.entries, |left, right| {
            match (left, right) {
                (Some(entry), None) | (None, Some(entry)) | (Some(entry), Some(_)) => {
                    entries.push(entry)
                }
                (None, None) => {}
            }
        });
        self.with_entries(entries)
    }

    fn intersection(&self, other: &Self) -> Self {
        let mut entries = Vec::with_capacity(self.len().min(other.len()));
        merge_entries(&self.entries, &other.entries, |left, right| {
            if let (Some(entry), Some(_)) = (left, right) {
                entries.push(entry);
            }
        });
        self.with_entries(entries)
    }

    fn difference(&self, other: &Self) -> Self {
        let mut entries = Vec::with_capacity(self.len());
        merge_entries(&self.entries, &other.entries, |left, right| {
            if let (Some(entry), None) = (left, right) {
                entries.push(entry);
            }
        });
        self.with_entries(entries)
    }

    fn with_entries(&self, entries: Vec<(K::Id, CandidateRow<K>)>) -> Self {
        Self {
            graph_id: self.graph_id,
            generation: self.generation,
            layout: Arc::clone(&self.layout),
            entries: entries.into(),
        }
    }
}

impl CandidateSet<Node> {
    pub(crate) fn from_node_rows(
        graph: &SeleneGraph,
        entries: impl IntoIterator<Item = (NodeId, NodeRow)>,
    ) -> Self {
        let mut entries = entries
            .into_iter()
            .map(|(id, row)| (id, CandidateRow::new(row.get())))
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|entry| entry.0);
        entries.dedup_by_key(|entry| entry.0);
        Self::from_sorted_entries(
            graph.meta.graph_id,
            graph.meta.generation,
            &graph.layout,
            entries,
        )
    }

    // M04-PR02 Part 3 deletes this minimum trusted lower-row bridge after all
    // downstream consumers move to typed candidates or stable-ID resolvers.
    #[allow(dead_code)]
    pub(crate) fn trusted_rows(&self) -> impl Iterator<Item = (NodeId, NodeRow)> + '_ {
        self.entries
            .iter()
            .map(|(id, row)| (*id, NodeRow::new(row.raw)))
    }

    #[cfg(test)]
    pub(crate) fn layout_weak(&self) -> std::sync::Weak<LayoutToken> {
        Arc::downgrade(&self.layout)
    }
}

impl CandidateSet<Edge> {
    pub(crate) fn from_edge_rows(
        graph: &SeleneGraph,
        entries: impl IntoIterator<Item = (EdgeId, EdgeRow)>,
    ) -> Self {
        let mut entries = entries
            .into_iter()
            .map(|(id, row)| (id, CandidateRow::new(row.get())))
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|entry| entry.0);
        entries.dedup_by_key(|entry| entry.0);
        Self::from_sorted_entries(
            graph.meta.graph_id,
            graph.meta.generation,
            &graph.layout,
            entries,
        )
    }

    // M04-PR02 Part 3 deletion owner: see the node-side bridge above.
    #[allow(dead_code)]
    pub(crate) fn trusted_rows(&self) -> impl Iterator<Item = (EdgeId, EdgeRow)> + '_ {
        self.entries
            .iter()
            .map(|(id, row)| (*id, EdgeRow::new(row.raw)))
    }
}

impl SeleneGraph {
    /// Build candidates for every node alive in this immutable snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] if a live typed row has no stable
    /// node ID in the snapshot's inverse map.
    pub fn live_node_candidates(&self) -> GraphResult<CandidateSet<Node>> {
        let entries = self
            .node_store
            .alive_rows()
            .map(|row| {
                self.node_id_for_node_row(row)
                    .map(|id| (id, row))
                    .ok_or_else(|| GraphError::Inconsistent {
                        reason: format!("alive node row {} has no stable id", row.get()),
                    })
            })
            .collect::<GraphResult<Vec<_>>>()?;
        Ok(CandidateSet::from_node_rows(self, entries))
    }

    /// Build candidates for every edge alive in this immutable snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] if a live typed row has no stable
    /// edge ID in the snapshot's inverse map.
    pub fn live_edge_candidates(&self) -> GraphResult<CandidateSet<Edge>> {
        let entries = self
            .edge_store
            .alive_rows()
            .map(|row| {
                self.edge_id_for_edge_row(row)
                    .map(|id| (id, row))
                    .ok_or_else(|| GraphError::Inconsistent {
                        reason: format!("alive edge row {} has no stable id", row.get()),
                    })
            })
            .collect::<GraphResult<Vec<_>>>()?;
        Ok(CandidateSet::from_edge_rows(self, entries))
    }

    /// Return the union of two candidates bound to this exact snapshot identity.
    pub fn union_candidates<K: CandidateKind>(
        &self,
        left: &CandidateSet<K>,
        right: &CandidateSet<K>,
    ) -> CandidateSetResult<CandidateSet<K>> {
        left.validate_for(self)?;
        right.validate_for(self)?;
        Ok(left.union(right))
    }

    /// Return the intersection of candidates bound to this exact snapshot identity.
    pub fn intersect_candidates<K: CandidateKind>(
        &self,
        left: &CandidateSet<K>,
        right: &CandidateSet<K>,
    ) -> CandidateSetResult<CandidateSet<K>> {
        left.validate_for(self)?;
        right.validate_for(self)?;
        Ok(left.intersection(right))
    }

    /// Return IDs from `left` absent from `right`, requiring this exact snapshot identity.
    pub fn difference_candidates<K: CandidateKind>(
        &self,
        left: &CandidateSet<K>,
        right: &CandidateSet<K>,
    ) -> CandidateSetResult<CandidateSet<K>> {
        left.validate_for(self)?;
        right.validate_for(self)?;
        Ok(left.difference(right))
    }

    pub(crate) fn remint_layout(&mut self) {
        self.layout = SnapshotLayout::new();
    }

    #[cfg(test)]
    pub(crate) fn shares_layout_with(&self, other: &Self) -> bool {
        self.layout.ptr_eq(&other.layout)
    }
}

fn merge_entries<K: CandidateKind>(
    left: &[(K::Id, CandidateRow<K>)],
    right: &[(K::Id, CandidateRow<K>)],
    mut emit: impl FnMut(Option<(K::Id, CandidateRow<K>)>, Option<(K::Id, CandidateRow<K>)>),
) {
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() || right_index < right.len() {
        match (left.get(left_index), right.get(right_index)) {
            (Some(left_entry), Some(right_entry)) if left_entry.0 < right_entry.0 => {
                emit(Some(*left_entry), None);
                left_index += 1;
            }
            (Some(left_entry), Some(right_entry)) if right_entry.0 < left_entry.0 => {
                emit(None, Some(*right_entry));
                right_index += 1;
            }
            (Some(left_entry), Some(right_entry)) => {
                debug_assert_eq!(left_entry.1.raw, right_entry.1.raw);
                emit(Some(*left_entry), Some(*right_entry));
                left_index += 1;
                right_index += 1;
            }
            (Some(left_entry), None) => {
                emit(Some(*left_entry), None);
                left_index += 1;
            }
            (None, Some(right_entry)) => {
                emit(None, Some(*right_entry));
                right_index += 1;
            }
            (None, None) => break,
        }
    }
}
