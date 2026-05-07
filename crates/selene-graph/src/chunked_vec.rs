//! Chunked column vector primitive per spec 03 section 3.2.
//!
//! Values are grouped into 2048-element chunks. Cloning the column clones only
//! the chunk Arcs; overwrites use `Arc::make_mut` so a write clones only the
//! affected chunk when a snapshot still shares it.

use std::marker::PhantomData;
use std::sync::Arc;

/// Number of column entries per chunk.
pub const CHUNK_SIZE: usize = 2048;

/// Column storage split into independently clone-on-write chunks.
#[derive(Clone, Debug)]
pub struct ChunkedVec<T> {
    chunks: Vec<Arc<[T]>>,
    len: usize,
    _marker: PhantomData<T>,
}

impl<T: Clone> ChunkedVec<T> {
    /// Construct an empty column.
    #[must_use]
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            len: 0,
            _marker: PhantomData,
        }
    }

    /// Construct an empty column with enough chunk slots for `capacity`.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            chunks: Vec::with_capacity(capacity.div_ceil(CHUNK_SIZE)),
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
        let (chunk, offset) = locate(index);
        self.chunks.get(chunk).and_then(|chunk| chunk.get(offset))
    }

    /// Append `value` to the column.
    pub fn push(&mut self, value: T) {
        match self.chunks.last_mut() {
            Some(last) if last.len() < CHUNK_SIZE => {
                let mut values = Vec::with_capacity(last.len() + 1);
                values.extend(last.iter().cloned());
                values.push(value);
                *last = Arc::from(values);
            }
            _ => {
                self.chunks.push(Arc::from(vec![value]));
            }
        }
        self.len += 1;
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
        let chunk = Arc::make_mut(&mut self.chunks[chunk_index]);
        chunk[offset] = value;
    }

    /// Iterate all values in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.chunks.iter().flat_map(|chunk| chunk.iter())
    }

    #[cfg(test)]
    pub(crate) fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    #[cfg(test)]
    pub(crate) fn chunk_capacity(&self) -> usize {
        self.chunks.capacity()
    }

    #[cfg(test)]
    pub(crate) fn chunk_arc(&self, index: usize) -> Arc<[T]> {
        Arc::clone(&self.chunks[index])
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
        assert_eq!(vec.chunk_count(), 3);
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
