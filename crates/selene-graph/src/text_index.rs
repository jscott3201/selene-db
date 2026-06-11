//! Reusable BM25 postings indexes over graph node string properties.
//!
//! `TextIndex` is an in-memory maintained artifact for one `(label, property)`
//! pair. Durable state stores only the registration; postings are derived from
//! primary node rows at registration, recovery, compaction, and commit-time
//! maintenance. The exact scorer in [`crate::text_search`] remains the
//! correctness oracle; this module reuses the same tokenizer, IDF formula, and
//! top-k ordering.

use std::collections::BTreeSet;
use std::mem::size_of;

use roaring::RoaringBitmap;
use rustc_hash::FxHashMap;

use selene_core::{CancellationChecker, DbString, LabelSet, NodeId, PropertyMap, Value};

use crate::error::{GraphError, GraphResult};
use crate::graph::{SeleneGraph, TextIndexEntry};
use crate::shared::SharedGraph;
use crate::store::RowIndex;
use crate::text_search::{
    DocumentStats, TextSearchError, TextSearchHit, TextTopK, bm25_score, tokenize_borrowed,
    unique_query_terms,
};

#[path = "text_index/candidate.rs"]
mod candidate;
/// In-memory BM25 postings index for one node `(label, property)` pair.
#[derive(Clone, Debug)]
pub struct TextIndex {
    label: DbString,
    property: DbString,
    rows: RoaringBitmap,
    document_lengths: FxHashMap<NodeId, u32>,
    document_terms: FxHashMap<NodeId, Vec<String>>,
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
    pub fn build(graph: &SeleneGraph, label: DbString, property: DbString) -> GraphResult<Self> {
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
    pub fn empty(label: DbString, property: DbString) -> Self {
        Self {
            label,
            property,
            rows: RoaringBitmap::new(),
            document_lengths: FxHashMap::default(),
            document_terms: FxHashMap::default(),
            postings: FxHashMap::default(),
            total_document_len: 0,
            posting_count: 0,
        }
    }

    /// Return the indexed node label.
    #[must_use]
    pub const fn label(&self) -> &DbString {
        &self.label
    }

    /// Return the indexed node property.
    #[must_use]
    pub const fn property(&self) -> &DbString {
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
        let mut document_term_bytes = self
            .document_terms
            .capacity()
            .saturating_mul(size_of::<(NodeId, Vec<String>)>());
        for terms in self.document_terms.values() {
            document_term_bytes = document_term_bytes
                .saturating_add(terms.capacity().saturating_mul(size_of::<String>()));
            for term in terms {
                document_term_bytes = document_term_bytes.saturating_add(term.capacity());
            }
        }
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
            .saturating_add(document_term_bytes)
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
            document_term_bytes,
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

    pub(crate) fn insert_document(&mut self, row: u32, node_id: NodeId, text: &str) {
        self.remove_document(row, node_id);
        let mut counts: FxHashMap<String, u32> = FxHashMap::default();
        let mut len = 0_u32;
        for token in tokenize_borrowed(text) {
            len = len.saturating_add(1);
            let count = counts.entry(token.into_owned()).or_insert(0);
            *count = count.saturating_add(1);
        }
        if len == 0 {
            return;
        }

        self.rows.insert(row);
        self.document_lengths.insert(node_id, len);
        self.total_document_len = self.total_document_len.saturating_add(u64::from(len));
        let mut terms = Vec::with_capacity(counts.len());
        for (term, term_count) in counts {
            let postings = self.postings.entry(term.clone()).or_default();
            match postings.binary_search_by_key(&node_id, |posting| posting.node_id) {
                Ok(index) => {
                    postings[index].term_count = term_count;
                }
                Err(index) => {
                    postings.insert(
                        index,
                        TextPosting {
                            node_id,
                            term_count,
                        },
                    );
                    self.posting_count = self.posting_count.saturating_add(1);
                }
            }
            terms.push(term);
        }
        self.document_terms.insert(node_id, terms);
    }

    pub(crate) fn remove_document(&mut self, row: u32, node_id: NodeId) {
        self.rows.remove(row);
        let Some(length) = self.document_lengths.remove(&node_id) else {
            return;
        };
        self.total_document_len = self.total_document_len.saturating_sub(u64::from(length));
        let Some(terms) = self.document_terms.remove(&node_id) else {
            return;
        };
        for term in terms {
            let remove_term = if let Some(postings) = self.postings.get_mut(&term) {
                if let Ok(index) =
                    postings.binary_search_by_key(&node_id, |posting| posting.node_id)
                {
                    postings.remove(index);
                    self.posting_count = self.posting_count.saturating_sub(1);
                }
                postings.is_empty()
            } else {
                false
            };
            if remove_term {
                self.postings.remove(&term);
            }
        }
    }

    fn finish_bulk_load(&mut self) {
        for postings in self.postings.values_mut() {
            postings.sort_by_key(|posting| posting.node_id);
        }
    }

    pub(crate) fn rows_eq(&self, reference: &Self) -> bool {
        self.rows == reference.rows
            && self.document_lengths == reference.document_lengths
            && self.total_document_len == reference.total_document_len
            && self.posting_count == reference.posting_count
            && self.postings == reference.postings
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
    /// Estimated bytes for per-document term lists used by commit maintenance.
    pub document_term_bytes: usize,
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
    pub fn build_text_index(
        &self,
        label: &DbString,
        property: &DbString,
    ) -> GraphResult<TextIndex> {
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
        label: &DbString,
        property: &DbString,
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
    pub fn build_text_index(
        &self,
        label: &DbString,
        property: &DbString,
    ) -> GraphResult<TextIndex> {
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
        label: &DbString,
        property: &DbString,
        query: &str,
        k: usize,
    ) -> GraphResult<Vec<TextSearchHit>> {
        self.read()
            .indexed_text_search_nodes(label, property, query, k)
    }
}

type TextIndexMap = FxHashMap<(DbString, DbString), TextIndexEntry>;

pub(crate) fn apply_node_create(
    indexes: &mut TextIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
    node_id: NodeId,
) {
    for label in labels.iter() {
        for (property, value) in props.iter() {
            insert_commit(
                indexes,
                label.clone(),
                property.clone(),
                value,
                row,
                node_id,
            );
        }
    }
}

pub(crate) fn apply_node_delete(
    indexes: &mut TextIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
    node_id: NodeId,
) {
    for label in labels.iter() {
        for (property, value) in props.iter() {
            remove_commit(
                indexes,
                label.clone(),
                property.clone(),
                value,
                row,
                node_id,
            );
        }
    }
}

pub(crate) fn apply_node_update(
    indexes: &mut TextIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
    row: u32,
    node_id: NodeId,
) {
    let candidates = candidate_keys(indexes, old_labels, old_props, new_labels, new_props);
    for (label, property) in candidates {
        match (
            indexable_text(old_labels, old_props, &label, &property),
            indexable_text(new_labels, new_props, &label, &property),
        ) {
            (Some(old_text), Some(new_text)) if old_text == new_text => {}
            (Some(_), Some(new_text)) => {
                insert_commit(
                    indexes,
                    label.clone(),
                    property.clone(),
                    new_text,
                    row,
                    node_id,
                );
            }
            (Some(old_text), None) => {
                remove_commit(
                    indexes,
                    label.clone(),
                    property.clone(),
                    old_text,
                    row,
                    node_id,
                );
            }
            (None, Some(new_text)) => {
                insert_commit(
                    indexes,
                    label.clone(),
                    property.clone(),
                    new_text,
                    row,
                    node_id,
                );
            }
            (None, None) => {}
        }
    }
}

pub(crate) fn rebuild_text_indexes(graph: &mut SeleneGraph) -> GraphResult<()> {
    let registrations: Vec<((DbString, DbString), Option<DbString>)> = graph
        .text_index
        .iter()
        .map(|(key, entry)| (key.clone(), entry.name.clone()))
        .collect();
    graph.text_index.clear();
    for ((label, property), name) in registrations {
        let index = TextIndex::build(graph, label.clone(), property.clone())?;
        graph
            .text_index
            .insert((label, property), TextIndexEntry::new(index, name));
    }
    Ok(())
}

fn candidate_keys(
    indexes: &TextIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
) -> BTreeSet<(DbString, DbString)> {
    if indexes.is_empty() {
        return BTreeSet::new();
    }
    let mut labels: BTreeSet<DbString> = BTreeSet::new();
    labels.extend(old_labels.iter().cloned());
    labels.extend(new_labels.iter().cloned());

    let mut properties: BTreeSet<DbString> = BTreeSet::new();
    properties.extend(old_props.keys().cloned());
    properties.extend(new_props.keys().cloned());

    let mut candidates = BTreeSet::new();
    for label in &labels {
        for property in &properties {
            let key = (label.clone(), property.clone());
            if indexes.contains_key(&key) {
                candidates.insert(key);
            }
        }
    }
    candidates
}

fn indexable_text<'a>(
    labels: &LabelSet,
    props: &'a PropertyMap,
    label: &DbString,
    property: &DbString,
) -> Option<&'a str> {
    if !labels.contains(label) {
        return None;
    }
    match props.get(property) {
        Some(Value::String(text)) => Some(text.as_str()),
        _ => None,
    }
}

fn insert_commit(
    indexes: &mut TextIndexMap,
    label: DbString,
    property: DbString,
    value: impl TextValue,
    row: u32,
    node_id: NodeId,
) {
    let Some(text) = value.text() else {
        return;
    };
    if let Some(entry) = indexes.get_mut(&(label, property)) {
        std::sync::Arc::make_mut(&mut entry.index).insert_document(row, node_id, text);
    }
}

fn remove_commit(
    indexes: &mut TextIndexMap,
    label: DbString,
    property: DbString,
    value: impl TextValue,
    row: u32,
    node_id: NodeId,
) {
    if value.text().is_none() {
        return;
    }
    if let Some(entry) = indexes.get_mut(&(label, property)) {
        std::sync::Arc::make_mut(&mut entry.index).remove_document(row, node_id);
    }
}

trait TextValue {
    fn text(&self) -> Option<&str>;
}

impl TextValue for &Value {
    fn text(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text.as_str()),
            _ => None,
        }
    }
}

impl TextValue for &str {
    fn text(&self) -> Option<&str> {
        Some(self)
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
