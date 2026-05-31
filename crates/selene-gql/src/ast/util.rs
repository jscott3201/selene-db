//! Small AST container types with encoded cardinality invariants.

use std::{
    error::Error,
    fmt,
    ops::Deref,
    slice::{Iter, IterMut},
};

use serde::{Deserialize, Deserializer, Serialize};

/// Error returned when a cardinality-constrained AST vector is too short.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmptyVecError {
    /// Minimum item count expected by the wrapper.
    pub expected_min: usize,
    /// Item count found in the provided vector.
    pub found: usize,
}

impl fmt::Display for EmptyVecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "expected at least {} item(s), found {}",
            self.expected_min, self.found
        )
    }
}

impl Error for EmptyVecError {}

/// Vector wrapper that guarantees at least `MIN` elements.
///
/// The cardinality floor is encoded in the `MIN` const generic, so a single
/// implementation backs every minimum-length AST container. The constructor is
/// the only way to build the wrapper, so the invariant `len() >= MIN` holds for
/// the lifetime of any value (including across a serde round-trip, which routes
/// through the same fallible constructor). See [`NonEmpty`] / [`Vec2OrMore`].
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BoundedVec<T, const MIN: usize> {
    values: Vec<T>,
}

impl<T, const MIN: usize> BoundedVec<T, MIN> {
    /// Build a wrapper enforcing the `MIN`-element floor.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyVecError`] when `values` has fewer than `MIN` items.
    pub fn try_from_vec(values: Vec<T>) -> Result<Self, EmptyVecError> {
        let found = values.len();
        if found < MIN {
            Err(EmptyVecError {
                expected_min: MIN,
                found,
            })
        } else {
            Ok(Self { values })
        }
    }

    /// Return the wrapped values as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }

    /// Iterate over the wrapped values.
    pub fn iter(&self) -> Iter<'_, T> {
        self.values.iter()
    }

    /// Iterate mutably over the wrapped values.
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        self.values.iter_mut()
    }

    /// Return the first value.
    #[must_use]
    pub fn first(&self) -> &T {
        &self.values[0]
    }

    /// Return the number of wrapped values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Return true when the wrapper has no values.
    ///
    /// This is always false for a valid wrapper (`MIN >= 1` for every alias in
    /// use) and exists for slice-like ergonomics in generic code.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Consume the wrapper and return the inner vector.
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.values
    }
}

impl<'de, T, const MIN: usize> Deserialize<'de> for BoundedVec<T, MIN>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<T>::deserialize(deserializer)?;
        Self::try_from_vec(values).map_err(serde::de::Error::custom)
    }
}

impl<T, const MIN: usize> Deref for BoundedVec<T, MIN> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<'a, T, const MIN: usize> IntoIterator for &'a BoundedVec<T, MIN> {
    type IntoIter = Iter<'a, T>;
    type Item = &'a T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, const MIN: usize> IntoIterator for &'a mut BoundedVec<T, MIN> {
    type IntoIter = IterMut<'a, T>;
    type Item = &'a mut T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T, const MIN: usize> IntoIterator for BoundedVec<T, MIN> {
    type IntoIter = std::vec::IntoIter<T>;
    type Item = T;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

/// Vector wrapper that guarantees at least one element.
pub type NonEmpty<T> = BoundedVec<T, 1>;

/// Vector wrapper that guarantees at least two elements.
pub type Vec2OrMore<T> = BoundedVec<T, 2>;

#[cfg(test)]
mod tests {
    use super::{EmptyVecError, NonEmpty, Vec2OrMore};

    #[test]
    fn try_from_vec_rejects_empty() {
        assert_eq!(
            NonEmpty::<u8>::try_from_vec(vec![]),
            Err(EmptyVecError {
                expected_min: 1,
                found: 0,
            })
        );
    }

    #[test]
    fn try_from_vec_rejects_singleton_for_vec2_or_more() {
        assert_eq!(
            Vec2OrMore::try_from_vec(vec![1_u8]),
            Err(EmptyVecError {
                expected_min: 2,
                found: 1,
            })
        );
    }

    #[test]
    fn serde_round_trip_rejects_empty() {
        let payload = serde_json::to_string(&Vec::<u8>::new()).expect("serialize vec");
        assert!(serde_json::from_str::<NonEmpty<u8>>(&payload).is_err());
    }

    #[test]
    fn serde_round_trip_rejects_singleton_for_vec2_or_more() {
        let payload = serde_json::to_string(&vec![1_u8]).expect("serialize vec");
        assert!(serde_json::from_str::<Vec2OrMore<u8>>(&payload).is_err());
    }

    #[test]
    fn serde_round_trip_accepts_valid() {
        let value = NonEmpty::try_from_vec(vec![1_u8, 2]).expect("non-empty");
        let payload = serde_json::to_string(&value).expect("serialize wrapper");
        let decoded = serde_json::from_str::<NonEmpty<u8>>(&payload).expect("deserialize wrapper");
        assert_eq!(value, decoded);
    }

    #[test]
    fn iter_and_as_slice_smoke() {
        let value = Vec2OrMore::try_from_vec(vec![1_u8, 2, 3]).expect("two or more");
        assert_eq!(value.as_slice(), &[1, 2, 3]);
        assert_eq!(value.iter().copied().sum::<u8>(), 6);
        assert_eq!(*value.first(), 1);
        assert_eq!(value.len(), 3);
    }
}
