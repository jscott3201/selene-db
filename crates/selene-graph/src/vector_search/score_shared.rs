use selene_core::{CancellationChecker, IStr, NodeId, VectorMetric, VectorValue};

use crate::error::GraphResult;
use crate::shared::SharedGraph;

use super::{
    VectorCandidateSet, VectorNeighborDirection, VectorNeighborSearchOptions, VectorNodeSearchHit,
    VectorSearchError,
};

impl SharedGraph {
    /// Score explicit node candidates in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`crate::SeleneGraph::score_vector_nodes`].
    pub fn score_vector_nodes(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &[NodeId],
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.read()
            .score_vector_nodes(property, query, candidates, metric, k)
    }

    /// Lock-free read snapshot wrapper for
    /// [`crate::SeleneGraph::score_vector_nodes_checked`].
    pub fn score_vector_nodes_checked(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &[NodeId],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        self.read()
            .score_vector_nodes_checked(property, query, candidates, metric, k, checker)
    }

    /// Score one explicit node candidate set per query in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`crate::SeleneGraph::score_vector_nodes_batch`].
    pub fn score_vector_nodes_batch<C>(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        candidate_sets: &[C],
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<Vec<VectorNodeSearchHit>>>
    where
        C: AsRef<[NodeId]>,
    {
        self.read()
            .score_vector_nodes_batch(property, queries, candidate_sets, metric, k)
    }

    /// Lock-free read snapshot wrapper for
    /// [`crate::SeleneGraph::score_vector_nodes_batch_checked`].
    pub fn score_vector_nodes_batch_checked<C>(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        candidate_sets: &[C],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError>
    where
        C: AsRef<[NodeId]>,
    {
        self.read().score_vector_nodes_batch_checked(
            property,
            queries,
            candidate_sets,
            metric,
            k,
            checker,
        )
    }

    /// Score one canonical candidate set against one query in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`crate::SeleneGraph::score_vector_candidate_set`].
    pub fn score_vector_candidate_set(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &VectorCandidateSet,
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.read()
            .score_vector_candidate_set(property, query, candidates, metric, k)
    }

    /// Lock-free read snapshot wrapper for
    /// [`crate::SeleneGraph::score_vector_candidate_set_checked`].
    pub fn score_vector_candidate_set_checked(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &VectorCandidateSet,
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        self.read()
            .score_vector_candidate_set_checked(property, query, candidates, metric, k, checker)
    }

    /// Score one canonical candidate set per query in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`crate::SeleneGraph::score_vector_candidate_sets_batch`].
    pub fn score_vector_candidate_sets_batch(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        candidate_sets: &[VectorCandidateSet],
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<Vec<VectorNodeSearchHit>>> {
        self.read()
            .score_vector_candidate_sets_batch(property, queries, candidate_sets, metric, k)
    }

    /// Lock-free read snapshot wrapper for
    /// [`crate::SeleneGraph::score_vector_candidate_sets_batch_checked`].
    pub fn score_vector_candidate_sets_batch_checked(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        candidate_sets: &[VectorCandidateSet],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        self.read().score_vector_candidate_sets_batch_checked(
            property,
            queries,
            candidate_sets,
            metric,
            k,
            checker,
        )
    }

    /// Score vector-valued neighbors reached from one anchor in the current snapshot.
    pub fn score_vector_neighbors(
        &self,
        property: &IStr,
        query: &VectorValue,
        anchor: NodeId,
        options: VectorNeighborSearchOptions<'_>,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.read()
            .score_vector_neighbors(property, query, anchor, options)
    }

    /// Lock-free read snapshot wrapper for
    /// [`crate::SeleneGraph::score_vector_neighbors_checked`].
    pub fn score_vector_neighbors_checked(
        &self,
        property: &IStr,
        query: &VectorValue,
        anchor: NodeId,
        options: VectorNeighborSearchOptions<'_>,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        self.read()
            .score_vector_neighbors_checked(property, query, anchor, options, checker)
    }

    /// Score one anchor's vector-valued neighbors for each query in the current snapshot.
    pub fn score_vector_neighbors_batch(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        anchors: &[NodeId],
        options: VectorNeighborSearchOptions<'_>,
    ) -> GraphResult<Vec<Vec<VectorNodeSearchHit>>> {
        self.read()
            .score_vector_neighbors_batch(property, queries, anchors, options)
    }

    /// Lock-free read snapshot wrapper for
    /// [`crate::SeleneGraph::score_vector_neighbors_batch_checked`].
    pub fn score_vector_neighbors_batch_checked(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        anchors: &[NodeId],
        options: VectorNeighborSearchOptions<'_>,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        self.read()
            .score_vector_neighbors_batch_checked(property, queries, anchors, options, checker)
    }

    /// Expand one canonical root set per query, then score it in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`crate::SeleneGraph::score_vector_expanded_candidate_sets_batch`].
    pub fn score_vector_expanded_candidate_sets_batch(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        root_sets: &[VectorCandidateSet],
        options: VectorNeighborSearchOptions<'_>,
    ) -> GraphResult<Vec<Vec<VectorNodeSearchHit>>> {
        self.read()
            .score_vector_expanded_candidate_sets_batch(property, queries, root_sets, options)
    }

    /// Lock-free read snapshot wrapper for
    /// [`crate::SeleneGraph::score_vector_expanded_candidate_sets_batch_checked`].
    pub fn score_vector_expanded_candidate_sets_batch_checked(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        root_sets: &[VectorCandidateSet],
        options: VectorNeighborSearchOptions<'_>,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        self.read()
            .score_vector_expanded_candidate_sets_batch_checked(
                property, queries, root_sets, options, checker,
            )
    }

    /// Return canonical vector-score candidates reached from one graph anchor
    /// in the current snapshot.
    #[must_use]
    pub fn vector_neighbor_candidates(
        &self,
        anchor: NodeId,
        edge_label: &IStr,
        direction: VectorNeighborDirection,
    ) -> VectorCandidateSet {
        self.read()
            .vector_neighbor_candidates(anchor, edge_label, direction)
    }

    /// Expand canonical candidates through one labeled graph hop in the current snapshot.
    #[must_use]
    pub fn expand_vector_candidate_set(
        &self,
        roots: &VectorCandidateSet,
        edge_label: &IStr,
        direction: VectorNeighborDirection,
    ) -> VectorCandidateSet {
        self.read()
            .expand_vector_candidate_set(roots, edge_label, direction)
    }

    /// Lock-free read snapshot wrapper for
    /// [`crate::SeleneGraph::expand_vector_candidate_set_checked`].
    pub fn expand_vector_candidate_set_checked(
        &self,
        roots: &VectorCandidateSet,
        edge_label: &IStr,
        direction: VectorNeighborDirection,
        checker: CancellationChecker<'_>,
    ) -> Result<VectorCandidateSet, VectorSearchError> {
        self.read()
            .expand_vector_candidate_set_checked(roots, edge_label, direction, checker)
    }
}
