//! Small support types and helpers for the in-memory IVF index.

use std::borrow::Cow;

use selene_core::{VectorTopK, VectorValue};

use super::{IvfVectorHit, MAX_CENTROIDS, TRAINING_SAMPLE_MAX_ENTRIES};

#[derive(Clone, Debug)]
pub(super) struct IvfEntry {
    pub(super) row: u32,
    pub(super) vector: VectorValue,
    pub(super) deleted: bool,
    pub(super) pending_retrain: bool,
}

pub(super) fn target_centroid_count(live_len: usize) -> usize {
    ceil_sqrt(live_len).clamp(1, MAX_CENTROIDS)
}

pub(super) fn training_entry_ids(live_entries: &[u32]) -> Cow<'_, [u32]> {
    if live_entries.len() <= TRAINING_SAMPLE_MAX_ENTRIES {
        return Cow::Borrowed(live_entries);
    }
    Cow::Owned(evenly_spaced_entry_ids(
        live_entries,
        TRAINING_SAMPLE_MAX_ENTRIES,
    ))
}

fn evenly_spaced_entry_ids(live_entries: &[u32], sample_len: usize) -> Vec<u32> {
    if sample_len == 0 || live_entries.is_empty() {
        return Vec::new();
    }
    if sample_len == 1 {
        return vec![live_entries[0]];
    }
    let last = live_entries.len() - 1;
    (0..sample_len)
        .map(|slot| {
            let source = slot.saturating_mul(last) / (sample_len - 1);
            live_entries[source]
        })
        .collect()
}

fn ceil_sqrt(value: usize) -> usize {
    let mut root = (value as f64).sqrt() as usize;
    while root.saturating_mul(root) < value {
        root += 1;
    }
    while root > 1 && (root - 1).saturating_mul(root - 1) >= value {
        root -= 1;
    }
    root
}

pub(super) fn average_list_len_basis_points(assigned_entries: usize, list_count: usize) -> usize {
    assigned_entries
        .saturating_mul(10_000)
        .checked_div(list_count)
        .unwrap_or_default()
}

pub(super) fn remove_entry_id(list: &mut Vec<u32>, entry_id: u32) -> bool {
    let Some(offset) = list.iter().position(|id| *id == entry_id) else {
        return false;
    };
    list.swap_remove(offset);
    true
}

pub(super) fn vector_hits(top_k: VectorTopK<u32>) -> Vec<IvfVectorHit> {
    top_k
        .into_hits()
        .into_iter()
        .map(|hit| IvfVectorHit {
            row: hit.key,
            distance: hit.distance,
        })
        .collect()
}
