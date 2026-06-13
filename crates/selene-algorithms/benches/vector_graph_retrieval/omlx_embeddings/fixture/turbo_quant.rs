//! TurboQuant rows for the local embedding pressure fixture.

use selene_core::{CancellationChecker, VectorMetric};
use selene_graph::ApproximateVectorSearchOptions;

use super::super::{TOP_K, TURBO_QUANT_SEARCH_WIDTH, precision_basis_points};
use super::OmlxVectorFixture;

impl OmlxVectorFixture {
    pub(in crate::fixture::omlx_embeddings) fn turbo_quant_total_precision(&self) -> usize {
        self.queries
            .iter()
            .map(|query| {
                let hits = self
                    .graph
                    .approximate_vector_search_nodes_checked(
                        &self.label,
                        &self.turbo_embedding_key,
                        &query.vector,
                        ApproximateVectorSearchOptions::new(
                            VectorMetric::Cosine,
                            TOP_K,
                            TURBO_QUANT_SEARCH_WIDTH,
                        ),
                        CancellationChecker::disabled(),
                    )
                    .expect("oMLX TurboQuant vector search succeeds");
                self.precision(query.topic, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum()
    }

    pub(in crate::fixture::omlx_embeddings) fn turbo_quant_precision_basis_points(&self) -> usize {
        precision_basis_points(
            self.turbo_quant_total_precision(),
            self.query_count() * TOP_K,
        )
    }
}
