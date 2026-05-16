//! Chunked column vector primitive per spec 03 section 3.2.
//!
//! Values are grouped into 2048-element chunks. Frozen chunks live as
//! `Arc<[T]>` and share cheaply across snapshots; the currently-filling chunk
//! is held as a mutable `Vec<T>` (the tail) so that `push` is amortized O(1).
//! Overwrites on frozen chunks use `Arc::make_mut`, cloning only the affected
//! chunk when a snapshot still holds it.

use std::marker::PhantomData;
use std::sync::Arc;

/// Number of column entries per chunk.
pub const CHUNK_SIZE: usize = 2048;

/// Column storage split into independently clone-on-write chunks.
#[derive(Clone, Debug)]
pub struct ChunkedVec<T> {
    chunks: Vec<Arc<[T]>>,
    tail: Vec<T>,
    len: usize,
    _marker: PhantomData<T>,
}

impl<T: Clone> ChunkedVec<T> {
    /// Construct an empty column.
    #[must_use]
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            tail: Vec::new(),
            len: 0,
            _marker: PhantomData,
        }
    }

    /// Construct an empty column with enough chunk slots for `capacity`.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            chunks: Vec::with_capacity(capacity.div_ceil(CHUNK_SIZE)),
            tail: Vec::new(),
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
    /// Pushes go into a mutable tail buffer with no Arc cloning. When the tail
    /// reaches `CHUNK_SIZE` it freezes into an `Arc<[T]>` immediately and a
    /// fresh tail starts on the next push. Cloning the column shares frozen
    /// chunks via Arc; the tail is duplicated cheaply (≤ `CHUNK_SIZE` elements).
    pub fn push(&mut self, value: T) {
        if self.tail.capacity() == 0 {
            self.tail.reserve(CHUNK_SIZE);
        }
        self.tail.push(value);
        self.len += 1;
        if self.tail.len() == CHUNK_SIZE {
            let frozen = std::mem::take(&mut self.tail);
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
            self.tail[offset] = value;
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
            Arc::from(self.tail.clone())
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
    }
}
