//! Lock-free snapshot wrappers for vector search APIs.

use selene_core::{CancellationChecker, DbString, VectorMetric, VectorValue};

use super::{
    ApproximateVectorExpansionOptions, ApproximateVectorSearchOptions, VectorNodeSearchHit,
    VectorSearchError,
};
use crate::error::GraphResult;
use crate::shared::SharedGraph;

impl SharedGraph {
    /// Exhaustively rank vector-valued node properties in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`crate::SeleneGraph::exact_vector_search_nodes`], so the result is
    /// lock-free with respect to concurrent writers once the snapshot pointer is
    /// read.
    pub fn exact_vector_search_nodes(
        &self,
        label: &DbString,
        property: &DbString,
        query: &VectorValue,
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.read()
            .exact_vector_search_nodes(label, property, query, metric, k)
    }

    /// Exhaustively rank vector-valued node properties with cancellation checks.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`crate::SeleneGraph::exact_vector_search_nodes_checked`].
    pub fn exact_vector_search_nodes_checked(
        &self,
        label: &DbString,
        property: &DbString,
        query: &VectorValue,
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        self.read()
            .exact_vector_search_nodes_checked(label, property, query, metric, k, checker)
    }

    /// Lock-free read snapshot wrapper for exact vector batch search.
    pub fn exact_vector_search_nodes_batch_checked(
        &self,
        label: &DbString,
        property: &DbString,
        queries: &[VectorValue],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        self.read()
            .exact_vector_search_nodes_batch_checked(label, property, queries, metric, k, checker)
    }

    /// Approximately rank vector-valued node properties through an ANN index.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`crate::SeleneGraph::approximate_vector_search_nodes_checked`].
    pub fn approximate_vector_search_nodes_checked(
        &self,
        label: &DbString,
        property: &DbString,
        query: &VectorValue,
        options: ApproximateVectorSearchOptions,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        self.read()
            .approximate_vector_search_nodes_checked(label, property, query, options, checker)
    }

    /// Lock-free read snapshot wrapper for approximate vector batch search.
    pub fn approximate_vector_search_nodes_batch_checked(
        &self,
        label: &DbString,
        property: &DbString,
        queries: &[VectorValue],
        options: ApproximateVectorSearchOptions,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        self.read().approximate_vector_search_nodes_batch_checked(
            label, property, queries, options, checker,
        )
    }

    /// Lock-free read snapshot wrapper for ANN-root graph expansion.
    pub fn approximate_vector_search_expanded_candidates_checked(
        &self,
        label: &DbString,
        property: &DbString,
        query: &VectorValue,
        options: ApproximateVectorExpansionOptions<'_>,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        self.read()
            .approximate_vector_search_expanded_candidates_checked(
                label, property, query, options, checker,
            )
    }

    /// Lock-free read snapshot wrapper for batched ANN-root graph expansion.
    pub fn approximate_vector_search_expanded_candidates_batch_checked(
        &self,
        label: &DbString,
        property: &DbString,
        queries: &[VectorValue],
        options: ApproximateVectorExpansionOptions<'_>,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        self.read()
            .approximate_vector_search_expanded_candidates_batch_checked(
                label, property, queries, options, checker,
            )
    }
}
