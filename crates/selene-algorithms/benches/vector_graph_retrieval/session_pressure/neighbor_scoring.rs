//! Native neighbor-scoring rows for active-hint vector retrieval benchmarks.

use selene_core::{CancellationChecker, NodeId, VectorMetric};
use selene_graph::{VectorNeighborDirection, VectorNeighborSearchOptions};

use super::super::{MemoryRetrievalFixture, Query, RankPrior, RetrievalQuality};
use super::SessionStrategy;

impl MemoryRetrievalFixture {
    pub(super) fn session_neighbor_scoring_quality(
        &self,
        strategy: SessionStrategy,
    ) -> RetrievalQuality {
        debug_assert!(matches!(
            strategy,
            SessionStrategy::GraphSessionDependencyActiveFilter
        ));
        self.queries
            .iter()
            .map(|query| self.session_neighbor_query_quality(query))
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    pub(super) fn session_neighbor_scoring_total_coverage(
        &self,
        strategy: SessionStrategy,
    ) -> usize {
        self.session_neighbor_scoring_quality(strategy).coverage
    }

    pub(super) fn session_neighbor_batch_scoring_quality(
        &self,
        strategy: SessionStrategy,
    ) -> RetrievalQuality {
        debug_assert!(matches!(
            strategy,
            SessionStrategy::GraphSessionDependencyActiveFilter
        ));
        let queries = self
            .queries
            .iter()
            .map(|query| query.vector.clone())
            .collect::<Vec<_>>();
        let anchors = self
            .queries
            .iter()
            .map(|query| query.anchor)
            .collect::<Vec<_>>();
        let options = VectorNeighborSearchOptions::new(
            &self.depends_edge,
            VectorNeighborDirection::Outgoing,
            VectorMetric::Cosine,
            self.max_dependency_neighbor_count(),
        );
        let batch_hits = self
            .graph
            .score_vector_neighbors_batch_checked(
                &self.embedding_key,
                &queries,
                &anchors,
                options,
                CancellationChecker::disabled(),
            )
            .expect("bench batch neighbor scoring succeeds");

        self.queries
            .iter()
            .zip(batch_hits)
            .map(|(query, hits)| {
                let selected =
                    self.select_from_candidates(query, hits, true, RankPrior::None, true);
                self.selected_quality(query, selected)
            })
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    pub(super) fn session_neighbor_batch_scoring_total_coverage(
        &self,
        strategy: SessionStrategy,
    ) -> usize {
        self.session_neighbor_batch_scoring_quality(strategy)
            .coverage
    }

    fn session_neighbor_query_quality(&self, query: &Query) -> RetrievalQuality {
        let options = VectorNeighborSearchOptions::new(
            &self.depends_edge,
            VectorNeighborDirection::Outgoing,
            VectorMetric::Cosine,
            self.dependency_neighbor_count(query.anchor),
        );
        let hits = self
            .graph
            .score_vector_neighbors_checked(
                &self.embedding_key,
                &query.vector,
                query.anchor,
                options,
                CancellationChecker::disabled(),
            )
            .expect("bench neighbor scoring succeeds");
        let selected = self.select_from_candidates(query, hits, true, RankPrior::None, true);
        self.selected_quality(query, selected)
    }

    fn max_dependency_neighbor_count(&self) -> usize {
        self.queries
            .iter()
            .map(|query| self.dependency_neighbor_count(query.anchor))
            .max()
            .unwrap_or(0)
    }

    fn dependency_neighbor_count(&self, anchor: NodeId) -> usize {
        self.graph.outgoing_edges(anchor).map_or(0, |edges| {
            edges
                .iter()
                .filter(|edge| edge.label == self.depends_edge)
                .count()
        })
    }
}
