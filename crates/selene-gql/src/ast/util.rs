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

/// Vector wrapper that guarantees at least one element.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct NonEmpty<T> {
    values: Vec<T>,
}

impl<T> NonEmpty<T> {
    /// Build a non-empty vector wrapper.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyVecError`] when `values` is empty.
    pub fn try_from_vec(values: Vec<T>) -> Result<Self, EmptyVecError> {
        validate_min_len(values, 1).map(|values| Self { values })
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
    /// This is always false for a valid wrapper and exists for slice-like
    /// ergonomics in generic code.
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

impl<'de, T> Deserialize<'de> for NonEmpty<T>
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

impl<T> Deref for NonEmpty<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<'a, T> IntoIterator for &'a NonEmpty<T> {
    type IntoIter = Iter<'a, T>;
    type Item = &'a T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut NonEmpty<T> {
    type IntoIter = IterMut<'a, T>;
    type Item = &'a mut T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T> IntoIterator for NonEmpty<T> {
    type IntoIter = std::vec::IntoIter<T>;
    type Item = T;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

/// Vector wrapper that guarantees at least two elements.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Vec2OrMore<T> {
    values: Vec<T>,
}

impl<T> Vec2OrMore<T> {
    /// Build a vector wrapper with at least two values.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyVecError`] when `values` has fewer than two items.
    pub fn try_from_vec(values: Vec<T>) -> Result<Self, EmptyVecError> {
        validate_min_len(values, 2).map(|values| Self { values })
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
    /// This is always false for a valid wrapper and exists for slice-like
    /// ergonomics in generic code.
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

impl<'de, T> Deserialize<'de> for Vec2OrMore<T>
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

impl<T> Deref for Vec2OrMore<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<'a, T> IntoIterator for &'a Vec2OrMore<T> {
    type IntoIter = Iter<'a, T>;
    type Item = &'a T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Vec2OrMore<T> {
    type IntoIter = IterMut<'a, T>;
    type Item = &'a mut T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T> IntoIterator for Vec2OrMore<T> {
    type IntoIter = std::vec::IntoIter<T>;
    type Item = T;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

fn validate_min_len<T>(values: Vec<T>, expected_min: usize) -> Result<Vec<T>, EmptyVecError> {
    let found = values.len();
    if found < expected_min {
        Err(EmptyVecError {
            expected_min,
            found,
        })
    } else {
        Ok(values)
    }
}

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
