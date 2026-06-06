//! BM25 composition rows for the local oMLX vector fixture.

use selene_graph::VectorCandidateSet;

use super::build_support::QueryVector;
use super::{OmlxVectorFixture, TOP_K};

const BM25_PRUNE_WIDTH: usize = 8;

impl OmlxVectorFixture {
    pub(in super::super) fn topic_hint_expansion_bm25_total_precision(&self) -> usize {
        self.queries
            .iter()
            .map(|query| {
                let candidates = self.topic_hint_expansion_bm25_candidate_set(query, TOP_K);
                self.precision(query.topic, candidates.as_nodes().iter().copied())
            })
            .sum()
    }

    pub(in super::super) fn topic_hint_expansion_bm25_vector_total_precision(&self) -> usize {
        self.candidate_sets_total_precision(|fixture, query| {
            fixture.topic_hint_expansion_bm25_candidate_set(query, BM25_PRUNE_WIDTH)
        })
    }

    pub(in super::super) fn topic_hint_expansion_bm25_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            self.topic_hint_expansion_bm25_candidate_set(query, TOP_K)
                .len()
        })
    }

    pub(in super::super) fn topic_hint_expansion_bm25_vector_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            self.topic_hint_expansion_bm25_candidate_set(query, BM25_PRUNE_WIDTH)
                .len()
        })
    }

    fn topic_hint_expansion_bm25_candidate_set(
        &self,
        query: &QueryVector,
        k: usize,
    ) -> VectorCandidateSet {
        let expanded = self.topic_hint_expansion_set(query);
        VectorCandidateSet::from_nodes(
            self.text_index
                .search_candidates(&query.text, expanded.as_nodes(), k)
                .into_iter()
                .map(|hit| hit.node_id),
        )
    }
}
