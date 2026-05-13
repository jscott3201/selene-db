//! IVF posting-list storage.

use rkyv::{Archive, Deserialize, Serialize};
use selene_core::NodeId;

/// One encoded vector inside an IVF posting list.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PostingEntry {
    /// Source graph node ID.
    pub node_id: NodeId,
    /// Residual PQ code bytes.
    pub codes: Box<[u8]>,
    /// Approximate norm of `coarse_centroid + decoded_residual`.
    pub reconstructed_norm: Option<f32>,
}

/// One IVF posting list keyed by coarse centroid id.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PostingList {
    /// Coarse centroid id.
    pub centroid_id: u32,
    /// Entries assigned to this centroid, in insertion order.
    pub entries: Vec<PostingEntry>,
}

impl PostingList {
    /// Construct an empty posting list.
    #[must_use]
    pub const fn new(centroid_id: u32) -> Self {
        Self {
            centroid_id,
            entries: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, entry: PostingEntry) {
        self.entries.push(entry);
    }
}

pub(crate) fn empty_posting_lists(k_coarse: u32) -> Vec<PostingList> {
    (0..k_coarse).map(PostingList::new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posting_list_insertion_appends_in_order() {
        let mut list = PostingList::new(7);
        list.push(PostingEntry {
            node_id: NodeId::new(11),
            codes: vec![1, 2].into_boxed_slice(),
            reconstructed_norm: None,
        });
        list.push(PostingEntry {
            node_id: NodeId::new(12),
            codes: vec![3, 4].into_boxed_slice(),
            reconstructed_norm: None,
        });

        assert_eq!(list.entries[0].node_id, NodeId::new(11));
        assert_eq!(list.entries[1].node_id, NodeId::new(12));
    }
}
