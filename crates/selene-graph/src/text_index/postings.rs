use std::sync::Arc;

use rustc_hash::FxHashMap;

use selene_core::NodeId;

use super::{TextPosting, TextTerm};

pub(super) fn upsert_posting(
    postings_by_term: &mut FxHashMap<TextTerm, Arc<Vec<TextPosting>>>,
    posting_count: &mut usize,
    term: &TextTerm,
    node_id: NodeId,
    term_count: u32,
) {
    let postings = postings_by_term
        .entry(Arc::clone(term))
        .or_insert_with(|| Arc::new(Vec::new()));
    let postings = Arc::make_mut(postings);
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
            *posting_count = posting_count.saturating_add(1);
        }
    }
}

pub(super) fn remove_posting(
    postings_by_term: &mut FxHashMap<TextTerm, Arc<Vec<TextPosting>>>,
    posting_count: &mut usize,
    term: &TextTerm,
    node_id: NodeId,
) {
    let remove_term = if let Some(postings) = postings_by_term.get_mut(term.as_ref()) {
        let postings = Arc::make_mut(postings);
        if let Ok(index) = postings.binary_search_by_key(&node_id, |posting| posting.node_id) {
            postings.remove(index);
            *posting_count = posting_count.saturating_sub(1);
        }
        postings.is_empty()
    } else {
        false
    };
    if remove_term {
        postings_by_term.remove(term.as_ref());
    }
}
