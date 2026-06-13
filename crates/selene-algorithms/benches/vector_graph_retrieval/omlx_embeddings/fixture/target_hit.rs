//! Target-hit quality rows for the local embedding pressure fixture.

use selene_core::{CancellationChecker, NodeId, VectorMetric};
use selene_graph::ApproximateVectorSearchOptions;

use super::super::{
    ANN_SEARCH_WIDTH, IVF_SEARCH_WIDTH, TOP_K, TURBO_QUANT_SEARCH_WIDTH, precision_basis_points,
};
use super::OmlxVectorFixture;

impl OmlxVectorFixture {
    pub(in crate::fixture::omlx_embeddings) fn exact_target_hit_basis_points(
        &self,
    ) -> Option<usize> {
        let hits = self
            .queries
            .iter()
            .map(|query| {
                let hits = self
                    .graph
                    .exact_vector_search_nodes_checked(
                        &self.label,
                        &self.embedding_key,
                        &query.vector,
                        VectorMetric::Cosine,
                        TOP_K,
                        CancellationChecker::disabled(),
                    )
                    .expect("oMLX exact vector search succeeds");
                self.target_hit(query.target_key, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum();
        self.target_hit_basis_points(hits)
    }

    pub(in crate::fixture::omlx_embeddings) fn ann_target_hit_basis_points(&self) -> Option<usize> {
        self.approximate_target_hit_basis_points(&self.embedding_key, ANN_SEARCH_WIDTH)
    }

    pub(in crate::fixture::omlx_embeddings) fn ivf_target_hit_basis_points(&self) -> Option<usize> {
        self.approximate_target_hit_basis_points(&self.ivf_embedding_key, IVF_SEARCH_WIDTH)
    }

    pub(in crate::fixture::omlx_embeddings) fn turbo_quant_target_hit_basis_points(
        &self,
    ) -> Option<usize> {
        self.approximate_target_hit_basis_points(
            &self.turbo_embedding_key,
            TURBO_QUANT_SEARCH_WIDTH,
        )
    }

    fn approximate_target_hit_basis_points(
        &self,
        property: &selene_core::DbString,
        search_width: usize,
    ) -> Option<usize> {
        let hits = self
            .queries
            .iter()
            .map(|query| {
                let hits = self
                    .graph
                    .approximate_vector_search_nodes_checked(
                        &self.label,
                        property,
                        &query.vector,
                        ApproximateVectorSearchOptions::new(
                            VectorMetric::Cosine,
                            TOP_K,
                            search_width,
                        ),
                        CancellationChecker::disabled(),
                    )
                    .expect("oMLX approximate vector search succeeds");
                self.target_hit(query.target_key, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum();
        self.target_hit_basis_points(hits)
    }

    fn target_hit_basis_points(&self, hits: usize) -> Option<usize> {
        let target_queries = self
            .queries
            .iter()
            .filter(|query| query.target_key.is_some())
            .count();
        (target_queries > 0).then(|| precision_basis_points(hits, target_queries))
    }

    fn target_hit<I>(&self, expected: Option<&'static str>, hits: I) -> usize
    where
        I: IntoIterator<Item = NodeId>,
    {
        let Some(expected) = expected else {
            return 0;
        };
        hits.into_iter()
            .any(|node| self.target_by_node.get(&node) == Some(&expected)) as usize
    }
}
