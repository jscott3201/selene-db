//! Ordered floating-point keys for typed property indexes.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

/// Marker error returned when a raw float cannot be admitted to a not-NaN key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("NaN is not an indexable floating-point value")]
pub struct NotNanError;

/// `f32` wrapper with total ordering via [`f32::total_cmp`].
#[derive(Clone, Copy, Debug)]
pub struct NotNanF32(f32);

impl NotNanF32 {
    /// Construct an ordered f32 key.
    ///
    /// # Errors
    ///
    /// Returns [`NotNanError`] when `value` is NaN.
    pub fn new(value: f32) -> Result<Self, NotNanError> {
        if value.is_nan() {
            Err(NotNanError)
        } else {
            Ok(Self(value))
        }
    }

    /// Return the underlying `f32`.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl PartialEq for NotNanF32 {
    fn eq(&self, rhs: &Self) -> bool {
        self.0.to_bits() == rhs.0.to_bits()
    }
}

impl Eq for NotNanF32 {}

impl PartialOrd for NotNanF32 {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

impl Ord for NotNanF32 {
    fn cmp(&self, rhs: &Self) -> Ordering {
        self.0.total_cmp(&rhs.0)
    }
}

impl Hash for NotNanF32 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

/// `f64` wrapper with total ordering via [`f64::total_cmp`].
///
/// The constructor rejects NaN because NaN has no useful equality or range
/// semantics for a graph property index. `+0.0` and `-0.0` remain distinct
/// keys because equality and hashing use the underlying bit pattern.
#[derive(Clone, Copy, Debug)]
pub struct NotNanF64(f64);

impl NotNanF64 {
    /// Construct a finite-or-infinite ordered f64 key.
    ///
    /// # Errors
    ///
    /// Returns [`NotNanError`] when `value` is NaN.
    pub fn new(value: f64) -> Result<Self, NotNanError> {
        if value.is_nan() {
            Err(NotNanError)
        } else {
            Ok(Self(value))
        }
    }

    /// Return the underlying `f64`.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl PartialEq for NotNanF64 {
    fn eq(&self, rhs: &Self) -> bool {
        self.0.to_bits() == rhs.0.to_bits()
    }
}

impl Eq for NotNanF64 {}

impl PartialOrd for NotNanF64 {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

impl Ord for NotNanF64 {
    fn cmp(&self, rhs: &Self) -> Ordering {
        self.0.total_cmp(&rhs.0)
    }
}

impl Hash for NotNanF64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}
