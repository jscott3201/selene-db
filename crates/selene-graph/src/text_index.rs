//! Reusable BM25 postings indexes over graph node string properties.
//!
//! `TextIndex` is an in-memory snapshot artifact: it is built from one
//! `(label, property)` pair and can answer repeated BM25 queries without scanning
//! every matching node. The exact scorer in [`crate::text_search`] remains the
//! correctness oracle; this module reuses the same tokenizer, IDF formula, and
//! top-k ordering.

use std::mem::size_of;

use roaring::RoaringBitmap;
use rustc_hash::FxHashMap;

use selene_core::{CancellationChecker, IStr, NodeId, Value};

use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::shared::SharedGraph;
use crate::store::RowIndex;
use crate::text_search::{
    DocumentStats, TextSearchError, TextSearchHit, TextTopK, bm25_score, tokenize,
    unique_query_terms,
};

/// In-memory BM25 postings index for one node `(label, property)` pair.
#[derive(Clone, Debug)]
pub struct TextIndex {
    label: IStr,
    property: IStr,
    rows: RoaringBitmap,
    document_lengths: FxHashMap<NodeId, u32>,
    postings: FxHashMap<String, Vec<TextPosting>>,
    total_document_len: u64,
    posting_count: usize,
}

impl TextIndex {
    /// Build a postings index from the current graph snapshot.
    ///
    /// Only alive nodes that carry `label` and whose `property` value is a
    /// non-empty string document are indexed. Non-string values are ignored, which
    /// matches the exact BM25 scan path.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] if the label index references a row
    /// without a resolvable node id or property row.
    pub fn build(graph: &SeleneGraph, label: IStr, property: IStr) -> GraphResult<Self> {
        let mut index = Self::empty(label.clone(), property.clone());
        let Some(label_rows) = graph.nodes_with_label(&label) else {
            return Ok(index);
        };

        for raw_row in label_rows.iter() {
            if !graph.node_store.is_alive(raw_row) {
                continue;
            }
            let row = RowIndex::new(raw_row);
            let node_id = graph
                .node_id_for_row(row)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "label index row {raw_row} for {} has no node id",
                        label.as_str()
                    ),
                })?;
            let properties = graph
                .node_store
                .properties
                .get(raw_row as usize)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "text index row {raw_row} for {} has no property row",
                        label.as_str()
                    ),
                })?;
            let Some(Value::String(text)) = properties.get(&property) else {
                continue;
            };
            index.insert_document(raw_row, node_id, text.as_str());
        }
        index.finish_bulk_load();
        Ok(index)
    }

    /// Construct an empty postings index for `label.property`.
    #[must_use]
    pub fn empty(label: IStr, property: IStr) -> Self {
        Self {
            label,
            property,
            rows: RoaringBitmap::new(),
            document_lengths: FxHashMap::default(),
            postings: FxHashMap::default(),
            total_document_len: 0,
            posting_count: 0,
        }
    }

    /// Return the indexed node label.
    #[must_use]
    pub const fn label(&self) -> &IStr {
        &self.label
    }

    /// Return the indexed node property.
    #[must_use]
    pub const fn property(&self) -> &IStr {
        &self.property
    }

    /// Return the indexed row bitmap.
    #[must_use]
    pub const fn rows(&self) -> &RoaringBitmap {
        &self.rows
    }

    /// Return the number of indexed string documents.
    #[must_use]
    pub fn document_count(&self) -> usize {
        self.document_lengths.len()
    }

    /// Return the number of distinct indexed terms.
    #[must_use]
    pub fn term_count(&self) -> usize {
        self.postings.len()
    }

    /// Return the total number of term-document postings.
    #[must_use]
    pub const fn posting_count(&self) -> usize {
        self.posting_count
    }

    /// Return aggregate index counters.
    #[must_use]
    pub fn stats(&self) -> TextIndexStats {
        TextIndexStats {
            indexed_rows: self.rows.len(),
            documents: self.document_count(),
            distinct_terms: self.term_count(),
            postings: self.posting_count,
            total_document_len: self.total_document_len,
        }
    }

    /// Return an estimated memory usage snapshot for this index.
    #[must_use]
    pub fn memory_usage(&self) -> TextIndexMemoryUsage {
        let row_bitmap_bytes = roaring_heap_bytes(&self.rows);
        let row_bitmap_serialized_bytes = self.rows.serialized_size();
        let document_length_bytes = self
            .document_lengths
            .capacity()
            .saturating_mul(size_of::<(NodeId, u32)>());
        let mut posting_bytes = 0usize;
        let mut term_bytes = 0usize;
        for (term, postings) in &self.postings {
            term_bytes = term_bytes.saturating_add(term.capacity());
            posting_bytes = posting_bytes
                .saturating_add(postings.capacity().saturating_mul(size_of::<TextPosting>()));
        }
        let terms_table_bytes = self
            .postings
            .capacity()
            .saturating_mul(size_of::<(String, Vec<TextPosting>)>());
        let estimated_index_bytes = size_of::<Self>()
            .saturating_add(row_bitmap_bytes)
            .saturating_add(document_length_bytes)
            .saturating_add(terms_table_bytes)
            .saturating_add(term_bytes)
            .saturating_add(posting_bytes);
        TextIndexMemoryUsage {
            indexed_rows: self.rows.len(),
            documents: self.document_count(),
            distinct_terms: self.term_count(),
            postings: self.posting_count,
            row_bitmap_bytes,
            row_bitmap_serialized_bytes,
            document_length_bytes,
            terms_table_bytes,
            term_bytes,
            posting_bytes,
            estimated_index_bytes,
        }
    }

    /// Rank indexed documents for `query` using BM25.
    #[must_use]
    pub fn search(&self, query: &str, k: usize) -> Vec<TextSearchHit> {
        self.search_checked(query, k, CancellationChecker::disabled())
            .expect("disabled text-index checker cannot fail")
    }

    /// Rank indexed documents for `query` with cooperative cancellation checks.
    ///
    /// The query path visits only postings for query terms, not every indexed
    /// document. Scores and tie ordering match the exact scan oracle.
    ///
    /// # Errors
    ///
    /// Returns [`TextSearchError::Cancelled`] or [`TextSearchError::Timeout`] when
    /// the supplied checker trips while collecting postings.
    pub fn search_checked(
        &self,
        query: &str,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<TextSearchHit>, TextSearchError> {
        checker.check()?;
        if k == 0 || self.document_lengths.is_empty() {
            return Ok(Vec::new());
        }
        let query_terms = unique_query_terms(query);
        if query_terms.is_empty() {
            return Ok(Vec::new());
        }

        let mut document_frequencies = vec![0_u32; query_terms.len()];
        let mut candidates: FxHashMap<NodeId, DocumentStats> = FxHashMap::default();
        let mut postings_since_check = 0usize;

        for (term_index, term) in query_terms.iter().enumerate() {
            let Some(postings) = self.postings.get(term) else {
                continue;
            };
            document_frequencies[term_index] = u32::try_from(postings.len()).unwrap_or(u32::MAX);
            for posting in postings {
                postings_since_check += 1;
                if postings_since_check >= crate::text_search::TEXT_SEARCH_CANCEL_STRIDE {
                    checker.check()?;
                    postings_since_check = 0;
                }
                let len = *self
                    .document_lengths
                    .get(&posting.node_id)
                    .expect("posting node must have document length");
                let doc = candidates.entry(posting.node_id).or_insert_with(|| {
                    DocumentStats::zero(posting.node_id, len, query_terms.len())
                });
                doc.term_counts[term_index] = posting.term_count;
            }
        }

        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let corpus_len = self.document_lengths.len() as f64;
        let average_document_len = self.total_document_len as f64 / corpus_len;
        let mut top_k = TextTopK::new(k);
        for doc in candidates.into_values() {
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

    fn insert_document(&mut self, row: u32, node_id: NodeId, text: &str) {
        let mut counts: FxHashMap<String, u32> = FxHashMap::default();
        let mut len = 0_u32;
        for token in tokenize(text) {
            len = len.saturating_add(1);
            *counts.entry(token).or_insert(0) += 1;
        }
        if len == 0 {
            return;
        }

        self.rows.insert(row);
        self.document_lengths.insert(node_id, len);
        self.total_document_len = self.total_document_len.saturating_add(u64::from(len));
        for (term, term_count) in counts {
            self.postings.entry(term).or_default().push(TextPosting {
                node_id,
                term_count,
            });
            self.posting_count = self.posting_count.saturating_add(1);
        }
    }

    fn finish_bulk_load(&mut self) {
        for postings in self.postings.values_mut() {
            postings.sort_by_key(|posting| posting.node_id);
        }
    }
}

/// Aggregate counters for a [`TextIndex`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextIndexStats {
    /// Number of indexed graph rows.
    pub indexed_rows: u64,
    /// Number of string documents with at least one token.
    pub documents: usize,
    /// Number of distinct terms in the vocabulary.
    pub distinct_terms: usize,
    /// Number of term-document postings.
    pub postings: usize,
    /// Sum of token counts across indexed documents.
    pub total_document_len: u64,
}

/// Estimated memory usage for one [`TextIndex`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextIndexMemoryUsage {
    /// Number of indexed graph rows.
    pub indexed_rows: u64,
    /// Number of string documents with at least one token.
    pub documents: usize,
    /// Number of distinct terms in the vocabulary.
    pub distinct_terms: usize,
    /// Number of term-document postings.
    pub postings: usize,
    /// Estimated heap bytes used by the row bitmap containers.
    pub row_bitmap_bytes: usize,
    /// Serialized byte size of the row bitmap.
    pub row_bitmap_serialized_bytes: usize,
    /// Estimated bytes for the document-length map.
    pub document_length_bytes: usize,
    /// Estimated bytes for the postings hash table.
    pub terms_table_bytes: usize,
    /// Estimated bytes for term string buffers.
    pub term_bytes: usize,
    /// Estimated bytes for posting vectors.
    pub posting_bytes: usize,
    /// Estimated bytes reachable from the index object.
    pub estimated_index_bytes: usize,
}

impl SeleneGraph {
    /// Build a reusable BM25 postings index for `label.property`.
    ///
    /// The returned index is tied to this graph snapshot. Mutations committed
    /// after the snapshot is read require rebuilding or durable registration in a
    /// later maintained-index layer.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] if graph label/property columns are
    /// internally inconsistent while the snapshot is scanned.
    pub fn build_text_index(&self, label: &IStr, property: &IStr) -> GraphResult<TextIndex> {
        TextIndex::build(self, label.clone(), property.clone())
    }

    /// Rank string-valued node properties through a transient postings index.
    ///
    /// This is primarily useful for tests and benchmark comparisons. Repeated
    /// production queries should build a [`TextIndex`] once and call
    /// [`TextIndex::search`] directly.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] if index construction observes corrupt
    /// graph columns.
    pub fn indexed_text_search_nodes(
        &self,
        label: &IStr,
        property: &IStr,
        query: &str,
        k: usize,
    ) -> GraphResult<Vec<TextSearchHit>> {
        Ok(self.build_text_index(label, property)?.search(query, k))
    }
}

impl SharedGraph {
    /// Build a reusable BM25 postings index from the current shared snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] if index construction observes corrupt
    /// graph columns.
    pub fn build_text_index(&self, label: &IStr, property: &IStr) -> GraphResult<TextIndex> {
        self.read().build_text_index(label, property)
    }

    /// Rank string-valued node properties through a transient postings index.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] if index construction observes corrupt
    /// graph columns.
    pub fn indexed_text_search_nodes(
        &self,
        label: &IStr,
        property: &IStr,
        query: &str,
        k: usize,
    ) -> GraphResult<Vec<TextSearchHit>> {
        self.read()
            .indexed_text_search_nodes(label, property, query, k)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TextPosting {
    node_id: NodeId,
    term_count: u32,
}

fn roaring_heap_bytes(rows: &RoaringBitmap) -> usize {
    let statistics = rows.statistics();
    usize::try_from(
        statistics
            .n_bytes_array_containers
            .saturating_add(statistics.n_bytes_run_containers)
            .saturating_add(statistics.n_bytes_bitset_containers),
    )
    .unwrap_or(usize::MAX)
}

#[cfg(test)]
#[path = "text_index/tests.rs"]
mod tests;
