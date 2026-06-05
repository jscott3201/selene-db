//! Exact BM25 full-text search over graph node properties.
//!
//! This module is the full-text correctness oracle: it scans the current graph
//! snapshot, tokenizes string properties, computes BM25 document statistics for
//! the requested `(label, property)` surface, and returns a deterministic
//! top-`k` ranking. Future maintained or postings-backed text indexes should
//! use this path as their ordering and recall reference.

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap};
use std::time::Duration;

use selene_core::{CancellationCause, CancellationChecker, IStr, NodeId, Value};

use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::shared::SharedGraph;
use crate::store::RowIndex;

const TEXT_SEARCH_CANCEL_STRIDE: usize = 1024;
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

/// One BM25-ranked node hit.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSearchHit {
    /// Matched node id.
    pub node_id: NodeId,
    /// Higher-is-better BM25 score.
    pub score: f64,
}

/// Error returned by checked text-search APIs.
#[derive(Debug, thiserror::Error)]
pub enum TextSearchError {
    /// Graph storage or consistency failure.
    #[error(transparent)]
    Graph(#[from] GraphError),
    /// Caller requested cooperative cancellation.
    #[error("text search cancelled")]
    Cancelled,
    /// Statement deadline elapsed.
    #[error("text search timed out after {elapsed:?}")]
    Timeout {
        /// Wall-clock duration since the deadline elapsed.
        elapsed: Duration,
    },
}

impl TextSearchError {
    fn into_graph_error(self) -> GraphError {
        match self {
            Self::Graph(error) => error,
            Self::Cancelled | Self::Timeout { .. } => GraphError::Inconsistent {
                reason: format!("disabled text-search checker returned {self}"),
            },
        }
    }
}

impl From<CancellationCause> for TextSearchError {
    fn from(cause: CancellationCause) -> Self {
        match cause {
            CancellationCause::Cancelled => Self::Cancelled,
            CancellationCause::Timeout { elapsed } => Self::Timeout { elapsed },
        }
    }
}

impl SeleneGraph {
    /// Exhaustively rank string-valued node properties using BM25.
    ///
    /// This is the full-text correctness oracle and small-corpus path. It scans
    /// the row bitmap for `label`, skips nodes where `property` is absent or not
    /// a string, tokenizes documents with the built-in Unicode-aware tokenizer,
    /// and ranks matches with Okapi BM25 (`k1 = 1.2`, `b = 0.75`). Query tokens
    /// are deduplicated so repeated query terms do not overweight a document.
    pub fn exact_text_search_nodes(
        &self,
        label: &IStr,
        property: &IStr,
        query: &str,
        k: usize,
    ) -> GraphResult<Vec<TextSearchHit>> {
        self.exact_text_search_nodes_checked(
            label,
            property,
            query,
            k,
            CancellationChecker::disabled(),
        )
        .map_err(TextSearchError::into_graph_error)
    }

    /// Exhaustively rank string-valued node properties with cancellation checks.
    pub fn exact_text_search_nodes_checked(
        &self,
        label: &IStr,
        property: &IStr,
        query: &str,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<TextSearchHit>, TextSearchError> {
        checker.check()?;
        if k == 0 {
            return Ok(Vec::new());
        }
        let query_terms = unique_query_terms(query);
        if query_terms.is_empty() {
            return Ok(Vec::new());
        }
        let Some(label_rows) = self.nodes_with_label(label) else {
            return Ok(Vec::new());
        };

        let mut docs = Vec::new();
        let mut document_frequencies = vec![0_u32; query_terms.len()];
        let mut total_document_len = 0_u64;
        let mut rows_since_check = 0usize;

        for raw_row in label_rows.iter() {
            rows_since_check += 1;
            if rows_since_check >= TEXT_SEARCH_CANCEL_STRIDE {
                checker.check()?;
                rows_since_check = 0;
            }
            if !self.node_store.is_alive(raw_row) {
                continue;
            }
            let row = RowIndex::new(raw_row);
            let node_id = self
                .node_id_for_row(row)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "label index row {raw_row} for {} has no node id",
                        label.as_str()
                    ),
                })?;
            let properties = self
                .node_store
                .properties
                .get(raw_row as usize)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "text search row {raw_row} for {} has no property row",
                        label.as_str()
                    ),
                })?;
            let Some(Value::String(text)) = properties.get(property) else {
                continue;
            };
            if let Some(doc) = document_stats(node_id, text.as_str(), &query_terms) {
                for (frequency, count) in document_frequencies.iter_mut().zip(&doc.term_counts) {
                    if *count > 0 {
                        *frequency += 1;
                    }
                }
                total_document_len += u64::from(doc.len);
                docs.push(doc);
            }
        }

        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let corpus_len = docs.len() as f64;
        let average_document_len = total_document_len as f64 / corpus_len;
        let mut top_k = TextTopK::new(k);
        for doc in docs {
            let score = bm25_score(
                &doc,
                &document_frequencies,
                corpus_len,
                average_document_len,
            );
            if score > 0.0 {
                top_k.push(doc.node_id, score);
            }
        }
        Ok(top_k.into_hits())
    }
}

impl SharedGraph {
    /// Exhaustively rank string-valued node properties in the current snapshot.
    pub fn exact_text_search_nodes(
        &self,
        label: &IStr,
        property: &IStr,
        query: &str,
        k: usize,
    ) -> GraphResult<Vec<TextSearchHit>> {
        self.read()
            .exact_text_search_nodes(label, property, query, k)
    }

    /// Exhaustively rank string-valued node properties with cancellation checks.
    pub fn exact_text_search_nodes_checked(
        &self,
        label: &IStr,
        property: &IStr,
        query: &str,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<TextSearchHit>, TextSearchError> {
        self.read()
            .exact_text_search_nodes_checked(label, property, query, k, checker)
    }
}

#[derive(Debug)]
struct DocumentStats {
    node_id: NodeId,
    len: u32,
    term_counts: Vec<u32>,
}

fn unique_query_terms(query: &str) -> Vec<String> {
    let terms: BTreeSet<_> = tokenize(query).into_iter().collect();
    terms.into_iter().collect()
}

fn document_stats(node_id: NodeId, text: &str, query_terms: &[String]) -> Option<DocumentStats> {
    let mut term_counts = vec![0_u32; query_terms.len()];
    let mut len = 0_u32;
    for token in tokenize(text) {
        len = len.saturating_add(1);
        if let Ok(index) = query_terms.binary_search(&token) {
            term_counts[index] = term_counts[index].saturating_add(1);
        }
    }
    (len > 0).then_some(DocumentStats {
        node_id,
        len,
        term_counts,
    })
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn bm25_score(
    doc: &DocumentStats,
    document_frequencies: &[u32],
    corpus_len: f64,
    average_document_len: f64,
) -> f64 {
    let document_len = f64::from(doc.len);
    doc.term_counts
        .iter()
        .zip(document_frequencies)
        .filter(|(term_count, _)| **term_count > 0)
        .map(|(term_count, document_frequency)| {
            let term_count = f64::from(*term_count);
            let document_frequency = f64::from(*document_frequency);
            let idf =
                (1.0 + (corpus_len - document_frequency + 0.5) / (document_frequency + 0.5)).ln();
            let normalization = term_count
                + BM25_K1 * (1.0 - BM25_B + BM25_B * document_len / average_document_len);
            idf * (term_count * (BM25_K1 + 1.0)) / normalization
        })
        .sum()
}

#[derive(Debug)]
struct TextTopK {
    k: usize,
    heap: BinaryHeap<TextHeapEntry>,
}

impl TextTopK {
    fn new(k: usize) -> Self {
        Self {
            k,
            heap: BinaryHeap::new(),
        }
    }

    fn push(&mut self, node_id: NodeId, score: f64) {
        debug_assert!(score.is_finite(), "BM25 scores must be finite");
        if self.k == 0 {
            return;
        }
        let entry = TextHeapEntry { score, node_id };
        if self.heap.len() < self.k {
            self.heap.push(entry);
            return;
        }
        let Some(worst) = self.heap.peek() else {
            return;
        };
        if entry.cmp(worst).is_lt() {
            self.heap.pop();
            self.heap.push(entry);
        }
    }

    fn into_hits(self) -> Vec<TextSearchHit> {
        let mut hits: Vec<_> = self
            .heap
            .into_iter()
            .map(|entry| TextSearchHit {
                node_id: entry.node_id,
                score: entry.score,
            })
            .collect();
        hits.sort_by(compare_hit);
        hits
    }
}

#[derive(Debug)]
struct TextHeapEntry {
    score: f64,
    node_id: NodeId,
}

impl Eq for TextHeapEntry {}

impl PartialEq for TextHeapEntry {
    fn eq(&self, rhs: &Self) -> bool {
        self.score.to_bits() == rhs.score.to_bits() && self.node_id == rhs.node_id
    }
}

impl Ord for TextHeapEntry {
    fn cmp(&self, rhs: &Self) -> Ordering {
        rhs.score
            .total_cmp(&self.score)
            .then_with(|| self.node_id.cmp(&rhs.node_id))
    }
}

impl PartialOrd for TextHeapEntry {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

fn compare_hit(lhs: &TextSearchHit, rhs: &TextSearchHit) -> Ordering {
    rhs.score
        .total_cmp(&lhs.score)
        .then_with(|| lhs.node_id.cmp(&rhs.node_id))
}

#[cfg(test)]
#[path = "text_search/tests.rs"]
mod tests;
