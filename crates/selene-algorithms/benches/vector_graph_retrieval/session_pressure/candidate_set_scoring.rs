//! Candidate-set batch scoring rows for active-hint vector retrieval benchmarks.

use selene_core::{CancellationChecker, VectorMetric};
use selene_graph::VectorCandidateSet;

use super::super::{MemoryRetrievalFixture, RankPrior, RetrievalQuality};
use super::SessionStrategy;

impl MemoryRetrievalFixture {
    pub(super) fn session_candidate_set_batch_scoring_quality(
        &self,
        strategy: SessionStrategy,
    ) -> RetrievalQuality {
        let mut queries = Vec::with_capacity(self.query_count());
        let mut candidate_sets = Vec::with_capacity(self.query_count());
        let mut max_candidates = 0;
        for query in &self.queries {
            let candidates =
                VectorCandidateSet::from_nodes(self.session_candidates(query, strategy));
            max_candidates = max_candidates.max(candidates.len());
            queries.push(query.vector.clone());
            candidate_sets.push(candidates);
        }

        let batch_hits = self
            .graph
            .score_vector_candidate_sets_batch_checked(
                &self.embedding_key,
                &queries,
                &candidate_sets,
                VectorMetric::Cosine,
                max_candidates,
                CancellationChecker::disabled(),
            )
            .expect("bench candidate-set batch scoring succeeds");

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

    pub(super) fn session_candidate_set_batch_scoring_total_coverage(
        &self,
        strategy: SessionStrategy,
    ) -> usize {
        self.session_candidate_set_batch_scoring_quality(strategy)
            .coverage
    }
}
