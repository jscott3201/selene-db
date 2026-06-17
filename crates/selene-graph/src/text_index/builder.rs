use std::sync::Arc;

use roaring::RoaringBitmap;
use rustc_hash::FxHashMap;

use selene_core::{DbString, NodeId};

use super::{TextIndex, TextPosting, TextTerm};
use crate::text_search::tokenize_borrowed;

pub(super) struct TextIndexBuilder {
    label: DbString,
    property: DbString,
    rows: RoaringBitmap,
    document_lengths: FxHashMap<NodeId, u32>,
    document_terms: FxHashMap<NodeId, Vec<TextTerm>>,
    postings: FxHashMap<TextTerm, Vec<TextPosting>>,
    interned_terms: FxHashMap<TextTerm, TextTerm>,
    total_document_len: u64,
    posting_count: usize,
}

impl TextIndexBuilder {
    pub(super) fn empty(label: DbString, property: DbString) -> Self {
        Self::with_document_capacity(label, property, 0)
    }

    pub(super) fn with_document_capacity(
        label: DbString,
        property: DbString,
        document_capacity: usize,
    ) -> Self {
        Self {
            label,
            property,
            rows: RoaringBitmap::new(),
            document_lengths: FxHashMap::with_capacity_and_hasher(
                document_capacity,
                Default::default(),
            ),
            document_terms: FxHashMap::with_capacity_and_hasher(
                document_capacity,
                Default::default(),
            ),
            postings: FxHashMap::default(),
            interned_terms: FxHashMap::default(),
            total_document_len: 0,
            posting_count: 0,
        }
    }

    pub(super) fn insert_document(&mut self, row: u32, node_id: NodeId, text: &str) {
        let mut counts: FxHashMap<TextTerm, u32> = FxHashMap::default();
        let mut len = 0_u32;
        for token in tokenize_borrowed(text) {
            len = len.saturating_add(1);
            let term = self.intern_term(token.as_ref());
            let count = counts.entry(term).or_insert(0);
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
            let postings = self.postings.entry(Arc::clone(&term)).or_default();
            postings.push(TextPosting {
                node_id,
                term_count,
            });
            self.posting_count = self.posting_count.saturating_add(1);
            terms.push(term);
        }
        self.document_terms.insert(node_id, terms);
    }

    pub(super) fn finish(mut self) -> TextIndex {
        for postings in self.postings.values_mut() {
            postings.sort_by_key(|posting| posting.node_id);
        }
        self.document_lengths.shrink_to_fit();
        self.document_terms.shrink_to_fit();
        TextIndex {
            label: self.label,
            property: self.property,
            rows: self.rows,
            document_lengths: self.document_lengths,
            document_terms: self
                .document_terms
                .into_iter()
                .map(|(node_id, terms)| (node_id, Arc::from(terms)))
                .collect(),
            postings: self
                .postings
                .into_iter()
                .map(|(term, postings)| (term, Arc::new(postings)))
                .collect(),
            total_document_len: self.total_document_len,
            posting_count: self.posting_count,
        }
    }

    fn intern_term(&mut self, token: &str) -> TextTerm {
        if let Some(term) = self.interned_terms.get(token) {
            return Arc::clone(term);
        }
        let term = TextTerm::from(token);
        self.interned_terms
            .insert(Arc::clone(&term), Arc::clone(&term));
        term
    }
}
