//! Candidate-scoped BM25 scoring for maintained text indexes.

use rustc_hash::FxHashSet;

use selene_core::{CancellationChecker, NodeId};

use super::{TextIndex, TextPosting};
use crate::text_search::{
    DocumentStats, TEXT_SEARCH_CANCEL_STRIDE, TextSearchError, TextSearchHit, TextTopK, bm25_score,
    unique_query_terms,
};

impl TextIndex {
    /// Rank explicit node candidates for `query` using this index's BM25 corpus stats.
    ///
    /// Candidate ids are deduplicated and missing/non-indexed ids are ignored.
    /// Term document frequencies and average document length remain global to
    /// the maintained index, so this returns the same ordering as a full
    /// [`Self::search`] followed by candidate filtering when enough global hits
    /// are materialized.
    #[must_use]
    pub fn search_candidates(
        &self,
        query: &str,
        candidates: &[NodeId],
        k: usize,
    ) -> Vec<TextSearchHit> {
        self.search_candidates_checked(query, candidates, k, CancellationChecker::disabled())
            .expect("disabled text-index checker cannot fail")
    }

    /// Rank explicit node candidates for `query` with cooperative cancellation checks.
    ///
    /// # Errors
    ///
    /// Returns [`TextSearchError::Cancelled`] or [`TextSearchError::Timeout`] when
    /// the supplied checker trips while deduplicating or scoring candidates.
    pub fn search_candidates_checked(
        &self,
        query: &str,
        candidates: &[NodeId],
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<TextSearchHit>, TextSearchError> {
        checker.check()?;
        if k == 0 || candidates.is_empty() || self.document_lengths.is_empty() {
            return Ok(Vec::new());
        }
        let query_terms = unique_query_terms(query);
        if query_terms.is_empty() {
            return Ok(Vec::new());
        }

        let (document_frequencies, postings_by_term) = self.query_postings(&query_terms);
        if postings_by_term.iter().all(Option::is_none) {
            return Ok(Vec::new());
        }
        let candidate_set = self.indexed_candidate_set(candidates, checker)?;
        if candidate_set.is_empty() {
            return Ok(Vec::new());
        }

        let corpus_len = self.document_lengths.len() as f64;
        let average_document_len = self.total_document_len as f64 / corpus_len;
        let mut top_k = TextTopK::new(k);
        let mut candidates_since_check = 0usize;
        for node_id in candidate_set {
            candidates_since_check += 1;
            if candidates_since_check >= TEXT_SEARCH_CANCEL_STRIDE {
                checker.check()?;
                candidates_since_check = 0;
            }
            let len = *self
                .document_lengths
                .get(&node_id)
                .expect("candidate set contains only indexed documents");
            let Some(doc) = candidate_document_stats(node_id, len, &postings_by_term) else {
                continue;
            };
            let score = bm25_score(
                &doc,
                &document_frequencies,
                corpus_len,
                average_document_len,
            );
            if score > 0.0 {
                top_k.push(node_id, score);
            }
        }
        Ok(top_k.into_hits())
    }

    fn indexed_candidate_set(
        &self,
        candidates: &[NodeId],
        checker: CancellationChecker<'_>,
    ) -> Result<FxHashSet<NodeId>, TextSearchError> {
        let mut set = FxHashSet::default();
        let mut candidates_since_check = 0usize;
        for &candidate in candidates {
            candidates_since_check += 1;
            if candidates_since_check >= TEXT_SEARCH_CANCEL_STRIDE {
                checker.check()?;
                candidates_since_check = 0;
            }
            if self.document_lengths.contains_key(&candidate) {
                set.insert(candidate);
            }
        }
        Ok(set)
    }

    fn query_postings<'a>(
        &'a self,
        query_terms: &[String],
    ) -> (Vec<u32>, Vec<Option<&'a [TextPosting]>>) {
        let mut document_frequencies = Vec::with_capacity(query_terms.len());
        let mut postings_by_term = Vec::with_capacity(query_terms.len());
        for term in query_terms {
            match self.postings.get(term) {
                Some(postings) => {
                    document_frequencies.push(u32::try_from(postings.len()).unwrap_or(u32::MAX));
                    postings_by_term.push(Some(postings.as_slice()));
                }
                None => {
                    document_frequencies.push(0);
                    postings_by_term.push(None);
                }
            }
        }
        (document_frequencies, postings_by_term)
    }
}

fn candidate_document_stats(
    node_id: NodeId,
    len: u32,
    postings_by_term: &[Option<&[TextPosting]>],
) -> Option<DocumentStats> {
    let mut doc = DocumentStats::zero(node_id, len, postings_by_term.len());
    let mut matched = false;
    for (term_index, postings) in postings_by_term.iter().enumerate() {
        let Some(postings) = postings else {
            continue;
        };
        if let Ok(index) = postings.binary_search_by_key(&node_id, |posting| posting.node_id) {
            doc.term_counts[term_index] = postings[index].term_count;
            matched = true;
        }
    }
    matched.then_some(doc)
}
