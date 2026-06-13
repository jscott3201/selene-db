//! Link storage for the in-memory HNSW graph.

use std::mem::size_of;

use rustc_hash::FxHashMap;
/// Upper HNSW layers kept on each node; level 0 lives in [`LevelZeroLinks`].
pub(super) type HnswUpperLinkLayers = Vec<Vec<u32>>;

/// Level-0 HNSW links with a compact bulk-built representation and mutable overlay.
#[derive(Clone, Debug)]
pub(super) enum LevelZeroLinks {
    /// Mutable construction/update representation.
    Mutable(Vec<Vec<u32>>),
    /// Compact representation for bulk-built indexes plus per-node mutation overlay.
    Compact {
        len: usize,
        offsets: Vec<usize>,
        links: Vec<u32>,
        overlay: FxHashMap<u32, Vec<u32>>,
    },
}

impl Default for LevelZeroLinks {
    fn default() -> Self {
        Self::new()
    }
}

impl LevelZeroLinks {
    /// Construct an empty mutable level-0 link store.
    pub(super) fn new() -> Self {
        Self::Mutable(Vec::new())
    }

    /// Append an empty link list for a newly inserted HNSW node.
    pub(super) fn push_empty(&mut self) {
        self.push(Vec::new());
    }

    /// Append an explicit link list.
    #[cfg(test)]
    pub(super) fn push_for_test(&mut self, links: Vec<u32>) {
        self.push(links);
    }

    fn push(&mut self, links: Vec<u32>) {
        match self {
            Self::Mutable(layers) => layers.push(links),
            Self::Compact { len, overlay, .. } => {
                let node_id = u32::try_from(*len).expect("HNSW entry count fits u32");
                *len = len.saturating_add(1);
                overlay.insert(node_id, links);
            }
        }
    }

    /// Borrow the level-0 links for `node_id`.
    pub(super) fn get(&self, node_id: u32) -> &[u32] {
        let idx = node_id as usize;
        match self {
            Self::Mutable(layers) => layers.get(idx).map_or(&[], Vec::as_slice),
            Self::Compact {
                len,
                offsets,
                links,
                overlay,
            } => {
                if let Some(layer) = overlay.get(&node_id) {
                    return layer.as_slice();
                }
                if idx >= *len || idx + 1 >= offsets.len() {
                    return &[];
                }
                let start = offsets[idx];
                let end = offsets[idx + 1];
                &links[start..end]
            }
        }
    }

    /// Mutably borrow the level-0 links for `node_id`, copying compact links on write.
    pub(super) fn get_mut(&mut self, node_id: u32) -> &mut Vec<u32> {
        let idx = node_id as usize;
        match self {
            Self::Mutable(layers) => layers
                .get_mut(idx)
                .expect("HNSW node has a level-0 link slot"),
            Self::Compact {
                len,
                offsets,
                links,
                overlay,
            } => {
                if idx >= *len {
                    *len = idx.saturating_add(1);
                }
                overlay.entry(node_id).or_insert_with(|| {
                    if idx + 1 >= offsets.len() {
                        return Vec::new();
                    }
                    let start = offsets[idx];
                    let end = offsets[idx + 1];
                    links[start..end].to_vec()
                })
            }
        }
    }

    /// Replace the level-0 links for `node_id`.
    pub(super) fn replace(&mut self, node_id: u32, links: Vec<u32>) {
        *self.get_mut(node_id) = links;
    }

    /// Compact all current level-0 links into one contiguous arena.
    pub(super) fn compact(&mut self) {
        let len = self.len();
        let link_count = self.link_count();
        let mut offsets = Vec::with_capacity(len.saturating_add(1));
        let mut compact_links = Vec::with_capacity(link_count);
        offsets.push(0);
        for idx in 0..len {
            compact_links.extend_from_slice(self.get(idx as u32));
            offsets.push(compact_links.len());
        }
        *self = Self::Compact {
            len,
            offsets,
            links: compact_links,
            overlay: FxHashMap::default(),
        };
    }

    /// Apply `visit` to every level-0 link slice.
    pub(super) fn for_each(&self, mut visit: impl FnMut(&[u32])) {
        for idx in 0..self.len() {
            visit(self.get(idx as u32));
        }
    }

    /// Estimated heap bytes owned by level-0 link storage.
    pub(super) fn estimated_heap_bytes(&self) -> usize {
        match self {
            Self::Mutable(layers) => layers
                .capacity()
                .saturating_mul(size_of::<Vec<u32>>())
                .saturating_add(
                    layers
                        .iter()
                        .map(|layer| layer.capacity().saturating_mul(size_of::<u32>()))
                        .sum::<usize>(),
                ),
            Self::Compact {
                offsets,
                links,
                overlay,
                ..
            } => offsets
                .capacity()
                .saturating_mul(size_of::<usize>())
                .saturating_add(links.capacity().saturating_mul(size_of::<u32>()))
                .saturating_add(
                    overlay
                        .capacity()
                        .saturating_mul(size_of::<(u32, Vec<u32>)>()),
                )
                .saturating_add(
                    overlay
                        .values()
                        .map(|layer| layer.capacity().saturating_mul(size_of::<u32>()))
                        .sum::<usize>(),
                ),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Mutable(layers) => layers.len(),
            Self::Compact { len, .. } => *len,
        }
    }

    fn link_count(&self) -> usize {
        let mut count = 0usize;
        self.for_each(|links| {
            count = count.saturating_add(links.len());
        });
        count
    }
}
