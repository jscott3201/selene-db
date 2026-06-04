use selene_core::{
    CancellationChecker, CoreError, IStr, NodeId, Value, VectorMetric, VectorTopK, VectorValue,
};

use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::shared::SharedGraph;

use super::{
    VECTOR_SEARCH_CANCEL_STRIDE, VectorCandidateSet, VectorNeighborDirection,
    VectorNeighborSearchOptions, VectorNodeSearchHit, VectorSearchError, vector_node_hits,
};

impl SeleneGraph {
    /// Score an explicit node candidate set against one query vector.
    ///
    /// This is the graph-retrieval rerank primitive: callers can produce
    /// candidates from graph pattern matches, graph algorithms, or ANN indexes,
    /// then rank only those nodes by a vector-valued property. Candidate ids are
    /// deduplicated before scoring. Missing, deleted, and non-vector candidates
    /// are skipped to match normal live-snapshot visibility.
    pub fn score_vector_nodes(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &[NodeId],
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.score_vector_nodes_checked(
            property,
            query,
            candidates,
            metric,
            k,
            CancellationChecker::disabled(),
        )
        .map_err(VectorSearchError::into_graph_error)
    }

    /// Score explicit node candidates with cancellation checks.
    ///
    /// This preserves [`Self::score_vector_nodes`] ordering and visibility while
    /// checking `checker` before work begins and every 1024 unique candidates.
    pub fn score_vector_nodes_checked(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &[NodeId],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        checker.check()?;
        if k == 0 || candidates.is_empty() {
            return Ok(Vec::new());
        }

        let candidates = VectorCandidateSet::from_nodes(candidates.iter().copied());
        self.score_vector_candidate_set_after_initial_check(
            property,
            query,
            &candidates,
            metric,
            k,
            checker,
        )
    }

    /// Score one canonical node candidate set against one query vector.
    ///
    /// This is the zero-renormalization companion to
    /// [`Self::score_vector_nodes`]. Callers that already hold a
    /// [`VectorCandidateSet`] can avoid the extra sort/dedup pass while keeping
    /// the same live-snapshot visibility, metric, and hit ordering semantics.
    pub fn score_vector_candidate_set(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &VectorCandidateSet,
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.score_vector_candidate_set_checked(
            property,
            query,
            candidates,
            metric,
            k,
            CancellationChecker::disabled(),
        )
        .map_err(VectorSearchError::into_graph_error)
    }

    /// Score one canonical node candidate set with cancellation checks.
    pub fn score_vector_candidate_set_checked(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &VectorCandidateSet,
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        checker.check()?;
        if k == 0 || candidates.is_empty() {
            return Ok(Vec::new());
        }
        self.score_vector_candidate_set_after_initial_check(
            property, query, candidates, metric, k, checker,
        )
    }

    fn score_vector_candidate_set_after_initial_check(
        &self,
        property: &IStr,
        query: &VectorValue,
        candidates: &VectorCandidateSet,
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        checker.check()?;

        let scorer = metric.bind_query(query).map_err(GraphError::from)?;
        let mut top_k = VectorTopK::new(k);
        for (offset, node_id) in candidates.as_nodes().iter().copied().enumerate() {
            if offset % VECTOR_SEARCH_CANCEL_STRIDE == 0 {
                checker.check()?;
            }
            let Some(properties) = self.node_properties(node_id) else {
                continue;
            };
            let Some(Value::Vector(vector)) = properties.get(property) else {
                continue;
            };
            let distance = scorer.distance(vector).map_err(GraphError::from)?;
            top_k.push_distance(node_id, distance);
        }

        Ok(vector_node_hits(top_k))
    }

    /// Score one explicit candidate set for each query vector.
    ///
    /// The result position corresponds to the input query position. Candidate
    /// sets are independent and follow [`Self::score_vector_nodes`] semantics:
    /// each set is deduplicated, non-live or non-vector nodes are skipped, and
    /// hits are ordered by distance then node id. The method rejects mismatched
    /// query/candidate-set counts and mixed query dimensions before scoring.
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
        self.score_vector_nodes_batch_checked(
            property,
            queries,
            candidate_sets,
            metric,
            k,
            CancellationChecker::disabled(),
        )
        .map_err(VectorSearchError::into_graph_error)
    }

    /// Score batched explicit node candidates with cancellation checks.
    ///
    /// This preserves [`Self::score_vector_nodes_batch`] ordering and
    /// visibility while checking `checker` before batch validation and before
    /// each query's candidate set is scored.
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
        checker.check()?;
        validate_batch_inputs(queries, candidate_sets.len())?;
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        if k == 0 {
            return Ok(vec![Vec::new(); queries.len()]);
        }

        let mut batch_hits = Vec::with_capacity(queries.len());
        for (query, candidates) in queries.iter().zip(candidate_sets) {
            checker.check()?;
            batch_hits.push(self.score_vector_nodes_checked(
                property,
                query,
                candidates.as_ref(),
                metric,
                k,
                checker,
            )?);
        }
        Ok(batch_hits)
    }

    /// Score one canonical candidate set for each query vector.
    ///
    /// This is the batch companion to [`Self::score_vector_candidate_set`]. It
    /// preserves the generic batch scoring contract while avoiding a second
    /// normalization pass for callers that already hold canonical candidate
    /// sets.
    pub fn score_vector_candidate_sets_batch(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        candidate_sets: &[VectorCandidateSet],
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<Vec<VectorNodeSearchHit>>> {
        self.score_vector_candidate_sets_batch_checked(
            property,
            queries,
            candidate_sets,
            metric,
            k,
            CancellationChecker::disabled(),
        )
        .map_err(VectorSearchError::into_graph_error)
    }

    /// Score batched canonical candidate sets with cancellation checks.
    pub fn score_vector_candidate_sets_batch_checked(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        candidate_sets: &[VectorCandidateSet],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        checker.check()?;
        validate_batch_inputs(queries, candidate_sets.len())?;
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        if k == 0 {
            return Ok(vec![Vec::new(); queries.len()]);
        }

        let mut batch_hits = Vec::with_capacity(queries.len());
        for (query, candidates) in queries.iter().zip(candidate_sets) {
            checker.check()?;
            batch_hits.push(self.score_vector_candidate_set_checked(
                property, query, candidates, metric, k, checker,
            )?);
        }
        Ok(batch_hits)
    }

    /// Score vector-valued neighbors reached from one anchor through `edge_label`.
    ///
    /// This is the one-hop graph candidate-set companion to
    /// [`Self::score_vector_nodes`]. It derives candidates from the snapshot's
    /// directed adjacency, then applies the same dedupe, visibility, metric, and
    /// ordering rules as explicit candidate scoring.
    pub fn score_vector_neighbors(
        &self,
        property: &IStr,
        query: &VectorValue,
        anchor: NodeId,
        options: VectorNeighborSearchOptions<'_>,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.score_vector_neighbors_checked(
            property,
            query,
            anchor,
            options,
            CancellationChecker::disabled(),
        )
        .map_err(VectorSearchError::into_graph_error)
    }

    /// Score vector-valued neighbors with cancellation checks.
    pub fn score_vector_neighbors_checked(
        &self,
        property: &IStr,
        query: &VectorValue,
        anchor: NodeId,
        options: VectorNeighborSearchOptions<'_>,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        checker.check()?;
        if options.k == 0 {
            return Ok(Vec::new());
        }
        let candidates =
            self.vector_neighbor_candidates(anchor, options.edge_label, options.direction);
        self.score_vector_candidate_set_checked(
            property,
            query,
            &candidates,
            options.metric,
            options.k,
            checker,
        )
    }

    /// Score one anchor's vector-valued neighbors for each query vector.
    ///
    /// `queries[i]` is scored against neighbors derived from `anchors[i]`.
    /// Mismatched query/anchor counts and mixed query dimensions are rejected
    /// before scoring.
    pub fn score_vector_neighbors_batch(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        anchors: &[NodeId],
        options: VectorNeighborSearchOptions<'_>,
    ) -> GraphResult<Vec<Vec<VectorNodeSearchHit>>> {
        self.score_vector_neighbors_batch_checked(
            property,
            queries,
            anchors,
            options,
            CancellationChecker::disabled(),
        )
        .map_err(VectorSearchError::into_graph_error)
    }

    /// Score batched one-hop graph neighbors with cancellation checks.
    pub fn score_vector_neighbors_batch_checked(
        &self,
        property: &IStr,
        queries: &[VectorValue],
        anchors: &[NodeId],
        options: VectorNeighborSearchOptions<'_>,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        checker.check()?;
        validate_batch_inputs(queries, anchors.len())?;
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        if options.k == 0 {
            return Ok(vec![Vec::new(); queries.len()]);
        }
        let candidate_sets: Vec<_> = anchors
            .iter()
            .map(|anchor| {
                self.vector_neighbor_candidates(*anchor, options.edge_label, options.direction)
            })
            .collect();
        self.score_vector_candidate_sets_batch_checked(
            property,
            queries,
            &candidate_sets,
            options.metric,
            options.k,
            checker,
        )
    }

    /// Return canonical vector-score candidates reached from one graph anchor.
    ///
    /// Candidates are filtered by edge label and direction, sorted by
    /// [`NodeId`], and deduplicated. The returned set intentionally does not
    /// check vector property presence or node liveness; scoring APIs apply
    /// normal snapshot visibility when the set is consumed.
    #[must_use]
    pub fn vector_neighbor_candidates(
        &self,
        anchor: NodeId,
        edge_label: &IStr,
        direction: VectorNeighborDirection,
    ) -> VectorCandidateSet {
        let mut candidates = Vec::new();
        if matches!(
            direction,
            VectorNeighborDirection::Outgoing | VectorNeighborDirection::Both
        ) && let Some(entry) = self.outgoing_edges(anchor)
        {
            candidates.extend(
                entry
                    .iter()
                    .filter(|edge| &edge.label == edge_label)
                    .map(|edge| edge.neighbor),
            );
        }
        if matches!(
            direction,
            VectorNeighborDirection::Incoming | VectorNeighborDirection::Both
        ) && let Some(entry) = self.incoming_edges(anchor)
        {
            candidates.extend(
                entry
                    .iter()
                    .filter(|edge| &edge.label == edge_label)
                    .map(|edge| edge.neighbor),
            );
        }
        VectorCandidateSet::from_nodes(candidates)
    }
}

impl SharedGraph {
    /// Score explicit node candidates in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`SeleneGraph::score_vector_nodes`].
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
    /// [`SeleneGraph::score_vector_nodes_checked`].
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
    /// [`SeleneGraph::score_vector_nodes_batch`].
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
    /// [`SeleneGraph::score_vector_nodes_batch_checked`].
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
    /// [`SeleneGraph::score_vector_candidate_set`].
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
    /// [`SeleneGraph::score_vector_candidate_set_checked`].
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
    /// [`SeleneGraph::score_vector_candidate_sets_batch`].
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
    /// [`SeleneGraph::score_vector_candidate_sets_batch_checked`].
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
    /// [`SeleneGraph::score_vector_neighbors_checked`].
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
    /// [`SeleneGraph::score_vector_neighbors_batch_checked`].
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
}

fn validate_batch_inputs(
    queries: &[VectorValue],
    candidate_set_count: usize,
) -> Result<(), VectorSearchError> {
    if queries.len() != candidate_set_count {
        return Err(VectorSearchError::BatchLengthMismatch {
            queries: queries.len(),
            candidate_sets: candidate_set_count,
        });
    }
    let Some(first_query) = queries.first() else {
        return Ok(());
    };
    let first_dimension = first_query.dimension();
    for query in &queries[1..] {
        if query.dimension() != first_dimension {
            return Err(GraphError::from(CoreError::VectorDimensionMismatch {
                lhs: first_dimension,
                rhs: query.dimension(),
            })
            .into());
        }
    }
    Ok(())
}
