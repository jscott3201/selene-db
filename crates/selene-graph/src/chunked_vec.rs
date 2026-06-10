//! Chunked column vector primitive per spec 03 section 3.2.
//!
//! Values are grouped into 2048-element chunks. Frozen chunks live as
//! `Arc<[T]>` and share cheaply across snapshots; the currently-filling chunk
//! (the tail) is held as an `Arc<Vec<T>>` so that cloning the column shares
//! the tail by refcount bump instead of deep-copying its elements. Mutations
//! use `Arc::make_mut` throughout: a write clones only the affected frozen
//! chunk or the tail, and only when a snapshot still holds it, so a clone
//! taken before a mutation never observes the mutation (B1 tail COW).

use std::marker::PhantomData;
use std::sync::Arc;

/// Number of column entries per chunk.
pub const CHUNK_SIZE: usize = 2048;

/// Column storage split into independently clone-on-write chunks.
#[derive(Clone, Debug)]
pub struct ChunkedVec<T> {
    chunks: Vec<Arc<[T]>>,
    tail: Arc<Vec<T>>,
    len: usize,
    _marker: PhantomData<T>,
}

impl<T: Clone> ChunkedVec<T> {
    /// Construct an empty column.
    #[must_use]
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            tail: Arc::new(Vec::new()),
            len: 0,
            _marker: PhantomData,
        }
    }

    /// Construct an empty column with enough chunk slots for `capacity`.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            chunks: Vec::with_capacity(capacity.div_ceil(CHUNK_SIZE)),
            tail: Arc::new(Vec::new()),
            len: 0,
            _marker: PhantomData,
        }
    }

    /// Number of entries in the column.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Return true when the column contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return the value at `index`, if it exists.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        let (chunk_index, offset) = locate(index);
        if chunk_index < self.chunks.len() {
            self.chunks[chunk_index].get(offset)
        } else {
            self.tail.get(offset)
        }
    }

    /// Append `value` to the column in amortized O(1) time.
    ///
    /// Pushes go into the tail buffer through `Arc::make_mut`: while the tail
    /// Arc is unique this is a plain `Vec::push`; the first push after a clone
    /// pays one tail clone (≤ `CHUNK_SIZE - 1` elements) and subsequent pushes
    /// reuse the now-unique buffer. When the tail reaches `CHUNK_SIZE` it
    /// freezes into an `Arc<[T]>` immediately and a fresh tail starts on the
    /// next push. Cloning the column shares frozen chunks *and* the tail via
    /// Arc refcount bumps — no element is copied at clone time.
    pub fn push(&mut self, value: T) {
        let tail = Arc::make_mut(&mut self.tail);
        if tail.capacity() == 0 {
            tail.reserve(CHUNK_SIZE);
        }
        tail.push(value);
        self.len += 1;
        if tail.len() == CHUNK_SIZE {
            let frozen = std::mem::take(tail);
            self.chunks.push(Arc::from(frozen));
        }
    }

    /// Replace the value at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn set(&mut self, index: usize, value: T) {
        assert!(
            index < self.len,
            "ChunkedVec::set index {index} out of bounds for len {}",
            self.len
        );
        let (chunk_index, offset) = locate(index);
        if chunk_index < self.chunks.len() {
            let chunk = Arc::make_mut(&mut self.chunks[chunk_index]);
            chunk[offset] = value;
        } else {
            Arc::make_mut(&mut self.tail)[offset] = value;
        }
    }

    /// Iterate all values in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.chunks
            .iter()
            .flat_map(|chunk| chunk.iter())
            .chain(self.tail.iter())
    }

    #[cfg(test)]
    pub(crate) fn chunk_count(&self) -> usize {
        self.chunks.len() + if self.tail.is_empty() { 0 } else { 1 }
    }

    #[cfg(test)]
    pub(crate) fn chunk_capacity(&self) -> usize {
        self.chunks.capacity() + 1
    }

    #[cfg(test)]
    pub(crate) fn chunk_arc(&self, index: usize) -> Arc<[T]> {
        if index < self.chunks.len() {
            Arc::clone(&self.chunks[index])
        } else {
            Arc::from(self.tail.as_slice())
        }
    }

    #[cfg(test)]
    pub(crate) fn slow_equals(&self, other: &Self) -> bool
    where
        T: PartialEq,
    {
        self.iter().eq(other.iter())
    }
}

impl<T: Clone> Default for ChunkedVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

fn locate(index: usize) -> (usize, usize) {
    (index / CHUNK_SIZE, index % CHUNK_SIZE)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn new_is_empty() {
        let vec = ChunkedVec::<u64>::new();
        assert_eq!(vec.len(), 0);
        assert!(vec.is_empty());
        assert_eq!(vec.chunk_count(), 0);
    }

    #[test]
    fn push_grows_and_get_returns_pushed_values() {
        let mut vec = ChunkedVec::new();
        for value in 0..5000 {
            vec.push(value);
        }
        assert_eq!(vec.len(), 5000);
        for value in 0..5000 {
            assert_eq!(vec.get(value), Some(&value));
        }
        // 5000 = 2 frozen full chunks (4096) + 904 in tail.
        assert_eq!(vec.chunk_count(), 3);
    }

    #[test]
    fn push_does_not_clone_tail_per_call() {
        // Pushing into a non-full tail must NOT touch frozen chunks' Arcs.
        let mut vec = ChunkedVec::new();
        for value in 0..CHUNK_SIZE {
            vec.push(value);
        }
        // First chunk just froze; tail is empty.
        assert_eq!(vec.chunks.len(), 1);
        let first_chunk_handle = Arc::clone(&vec.chunks[0]);
        // Push 100 more into the (now-fresh) tail; the frozen chunk's Arc
        // strong count must stay 2 (vec.chunks[0] + first_chunk_handle).
        for value in 0..100 {
            vec.push(CHUNK_SIZE + value);
        }
        assert_eq!(Arc::strong_count(&first_chunk_handle), 2);
        assert_eq!(vec.len(), CHUNK_SIZE + 100);
    }

    #[test]
    fn set_replaces_value_at_index() {
        let mut vec = ChunkedVec::new();
        vec.push(1);
        vec.push(2);
        vec.set(1, 9);
        assert_eq!(vec.get(1), Some(&9));
    }

    #[test]
    fn set_only_clones_affected_chunk() {
        let mut original = ChunkedVec::new();
        for value in 0..4096 {
            original.push(value);
        }
        // Two full frozen chunks, empty tail.
        let mut cloned = original.clone();
        assert!(original.slow_equals(&cloned));
        let original_chunk_0 = original.chunk_arc(0);
        let original_chunk_1 = original.chunk_arc(1);
        cloned.set(CHUNK_SIZE + 1, 99);
        assert!(!original.slow_equals(&cloned));
        assert_eq!(Arc::strong_count(&original_chunk_0), 3);
        assert_eq!(Arc::strong_count(&original_chunk_1), 2);
        assert_eq!(original.get(CHUNK_SIZE + 1), Some(&(CHUNK_SIZE + 1)));
        assert_eq!(cloned.get(CHUNK_SIZE + 1), Some(&99));
    }

    #[test]
    fn set_in_tail_does_not_touch_frozen_chunks() {
        let mut vec = ChunkedVec::new();
        for value in 0..CHUNK_SIZE + 50 {
            vec.push(value);
        }
        let frozen_handle = Arc::clone(&vec.chunks[0]);
        vec.set(CHUNK_SIZE + 10, 999);
        // The set targeted the tail; frozen chunk's strong count stays 2.
        assert_eq!(Arc::strong_count(&frozen_handle), 2);
        assert_eq!(vec.get(CHUNK_SIZE + 10), Some(&999));
    }

    #[test]
    fn iter_yields_all_values_in_order() {
        let mut vec = ChunkedVec::new();
        for value in 0..4096 {
            vec.push(value);
        }
        assert_eq!(
            vec.iter().copied().collect::<Vec<_>>(),
            (0..4096).collect::<Vec<_>>()
        );
    }

    #[test]
    fn iter_includes_tail() {
        let mut vec = ChunkedVec::new();
        for value in 0..CHUNK_SIZE + 5 {
            vec.push(value);
        }
        assert_eq!(
            vec.iter().copied().collect::<Vec<_>>(),
            (0..CHUNK_SIZE + 5).collect::<Vec<_>>()
        );
    }

    #[test]
    fn with_capacity_does_not_overallocate_entries() {
        let vec = ChunkedVec::<u64>::with_capacity(CHUNK_SIZE * 3 + 1);
        assert_eq!(vec.len(), 0);
        assert_eq!(vec.chunk_count(), 0);
        assert!(vec.chunk_capacity() >= 4);
    }

    #[test]
    fn get_out_of_bounds_returns_none() {
        let mut vec = ChunkedVec::new();
        vec.push(1);
        assert_eq!(vec.get(1), None);
    }

    #[test]
    #[should_panic(expected = "ChunkedVec::set index 1 out of bounds for len 1")]
    fn set_out_of_bounds_panics_clearly() {
        let mut vec = ChunkedVec::new();
        vec.push(1);
        vec.set(1, 2);
    }

    #[test]
    fn clone_shares_tail_without_copying() {
        // B1: cloning the column must share the tail allocation by refcount
        // bump — read-only clones never duplicate tail elements.
        let mut original = ChunkedVec::new();
        for value in 0..100 {
            original.push(value);
        }
        let cloned = original.clone();
        assert!(Arc::ptr_eq(&original.tail, &cloned.tail));
        assert_eq!(Arc::strong_count(&original.tail), 2);
        assert!(original.slow_equals(&cloned));
    }

    #[test]
    fn clone_then_push_isolates_original_snapshot() {
        // B1: a push on the clone must COW the tail; the original (the
        // "snapshot") keeps its pre-mutation contents and its own tail Arc.
        let mut original = ChunkedVec::new();
        for value in 0..10 {
            original.push(value);
        }
        let snapshot_tail = Arc::clone(&original.tail);
        let mut cloned = original.clone();
        cloned.push(999);
        // Original snapshot unchanged.
        assert_eq!(original.len(), 10);
        assert_eq!(original.get(10), None);
        for value in 0..10 {
            assert_eq!(original.get(value), Some(&value));
        }
        // Clone diverged onto its own tail allocation.
        assert_eq!(cloned.get(10), Some(&999));
        assert!(!Arc::ptr_eq(&original.tail, &cloned.tail));
        // snapshot_tail + original.tail — the clone no longer shares it.
        assert_eq!(Arc::strong_count(&snapshot_tail), 2);
    }

    #[test]
    fn clone_then_set_in_tail_isolates_original_snapshot() {
        let mut original = ChunkedVec::new();
        for value in 0..10 {
            original.push(value);
        }
        let mut cloned = original.clone();
        cloned.set(3, 777);
        assert_eq!(original.get(3), Some(&3));
        assert_eq!(cloned.get(3), Some(&777));
        assert!(!Arc::ptr_eq(&original.tail, &cloned.tail));
    }

    #[test]
    fn first_write_makes_tail_unique_then_reuses_buffer() {
        // The first push after a clone pays the one tail COW; subsequent
        // pushes mutate the now-unique buffer in place (no further clones).
        let mut original = ChunkedVec::new();
        for value in 0..10 {
            original.push(value);
        }
        let mut cloned = original.clone();
        cloned.push(100);
        // Compare raw Arc allocation pointers — holding a probe Arc clone
        // would itself force `make_mut` to clone on the next write.
        let unique_tail_ptr = Arc::as_ptr(&cloned.tail);
        cloned.push(101);
        cloned.set(0, 42);
        // Still the same (unique) allocation: no further COW clones happened.
        assert_eq!(Arc::as_ptr(&cloned.tail), unique_tail_ptr);
        assert_eq!(Arc::strong_count(&cloned.tail), 1);
        assert_eq!(cloned.get(0), Some(&42));
        assert_eq!(original.get(0), Some(&0));
    }

    #[test]
    fn freeze_under_share_preserves_both_columns() {
        // Fill a clone up to the CHUNK_SIZE freeze boundary while the original
        // still holds the pre-freeze tail Arc. The freeze must COW first, so
        // the original keeps its short tail and the clone freezes correctly.
        let mut original = ChunkedVec::new();
        for value in 0..CHUNK_SIZE - 1 {
            original.push(value);
        }
        let mut cloned = original.clone();
        cloned.push(CHUNK_SIZE - 1); // triggers freeze in the clone
        for value in CHUNK_SIZE..CHUNK_SIZE + 5 {
            cloned.push(value);
        }
        // Original: one (unfrozen) tail of CHUNK_SIZE - 1 entries.
        assert_eq!(original.len(), CHUNK_SIZE - 1);
        assert_eq!(original.chunks.len(), 0);
        assert_eq!(original.tail.len(), CHUNK_SIZE - 1);
        for value in 0..CHUNK_SIZE - 1 {
            assert_eq!(original.get(value), Some(&value));
        }
        // Clone: one frozen chunk plus a 5-entry fresh tail.
        assert_eq!(cloned.len(), CHUNK_SIZE + 5);
        assert_eq!(cloned.chunks.len(), 1);
        assert_eq!(cloned.tail.len(), 5);
        for value in 0..CHUNK_SIZE + 5 {
            assert_eq!(cloned.get(value), Some(&value));
        }
    }

    proptest! {
        #[test]
        fn random_push_set_sequence_preserves_latest_values(ops in proptest::collection::vec((any::<bool>(), 0_usize..128, any::<u16>()), 1..256)) {
            let mut vec = ChunkedVec::new();
            let mut expected = Vec::new();
            for (set_existing, index, value) in ops {
                if set_existing && !expected.is_empty() {
                    let idx = index % expected.len();
                    vec.set(idx, value);
                    expected[idx] = value;
                } else {
                    vec.push(value);
                    expected.push(value);
                }
                prop_assert_eq!(vec.len(), expected.len());
                for (idx, expected_value) in expected.iter().enumerate() {
                    prop_assert_eq!(vec.get(idx), Some(expected_value));
                }
            }
        }

        #[test]
        fn clone_then_mutate_never_disturbs_snapshot(
            seed_ops in proptest::collection::vec((any::<bool>(), 0_usize..4096, any::<u16>()), 1..512),
            post_ops in proptest::collection::vec((any::<bool>(), 0_usize..4096, any::<u16>()), 1..512),
        ) {
            // B1 snapshot isolation: a clone taken at an arbitrary fill point
            // (including straddling chunk freezes) must never observe pushes
            // or sets applied to the live column afterwards.
            let mut live = ChunkedVec::new();
            let mut model = Vec::new();
            for (set_existing, index, value) in seed_ops {
                if set_existing && !model.is_empty() {
                    let idx = index % model.len();
                    live.set(idx, value);
                    model[idx] = value;
                } else {
                    live.push(value);
                    model.push(value);
                }
            }
            let snapshot = live.clone();
            let snapshot_model = model.clone();
            for (set_existing, index, value) in post_ops {
                if set_existing && !model.is_empty() {
                    let idx = index % model.len();
                    live.set(idx, value);
                    model[idx] = value;
                } else {
                    live.push(value);
                    model.push(value);
                }
            }
            // Snapshot still matches the pre-clone model exactly.
            prop_assert_eq!(snapshot.len(), snapshot_model.len());
            for (idx, expected_value) in snapshot_model.iter().enumerate() {
                prop_assert_eq!(snapshot.get(idx), Some(expected_value));
            }
            prop_assert_eq!(snapshot.get(snapshot_model.len()), None);
            // Live column matches the post-mutation model.
            prop_assert_eq!(live.len(), model.len());
            for (idx, expected_value) in model.iter().enumerate() {
                prop_assert_eq!(live.get(idx), Some(expected_value));
            }
        }
    }
}
