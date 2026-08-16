//! Ordered floating-point keys for typed property indexes.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

/// Marker error returned when a raw float cannot be admitted to a not-NaN key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("NaN is not an indexable floating-point value")]
pub struct NotNanError;

/// `f32` wrapper with total ordering via [`f32::total_cmp`].
///
/// See [`NotNanF64`] for the signed-zero and NaN admission rules, which are
/// identical here.
#[derive(Clone, Copy, Debug)]
pub struct NotNanF32(f32);

impl NotNanF32 {
    /// Construct an ordered f32 key, collapsing `-0.0` onto `+0.0`.
    ///
    /// # Errors
    ///
    /// Returns [`NotNanError`] when `value` is NaN.
    pub fn new(value: f32) -> Result<Self, NotNanError> {
        if value.is_nan() {
            Err(NotNanError)
        } else if value == 0.0 {
            Ok(Self(0.0))
        } else {
            Ok(Self(value))
        }
    }

    /// Return the key's `f32`, which is `+0.0` for either signed zero.
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
/// semantics for a graph property index.
///
/// It collapses `-0.0` onto `+0.0` because GQL compares the two zeros equal.
/// `total_cmp` does not: it orders `-0.0` strictly below `+0.0`, and the bit
/// patterns differ, so keying on the raw value would file the two zeros under
/// separate keys. An indexed `= 0.0` would then answer with only the rows that
/// happened to store the same sign, disagreeing with the identical unindexed
/// read while every row remained keyable and the index looked healthy.
///
/// Collapsing at construction covers range bounds too: `> -0.0` becomes
/// `> +0.0` and correctly excludes a `0.0` row, matching the scan.
#[derive(Clone, Copy, Debug)]
pub struct NotNanF64(f64);

impl NotNanF64 {
    /// Construct a finite-or-infinite ordered f64 key, collapsing `-0.0` onto
    /// `+0.0`.
    ///
    /// # Errors
    ///
    /// Returns [`NotNanError`] when `value` is NaN.
    pub fn new(value: f64) -> Result<Self, NotNanError> {
        if value.is_nan() {
            Err(NotNanError)
        } else if value == 0.0 {
            Ok(Self(0.0))
        } else {
            Ok(Self(value))
        }
    }

    /// Return the key's `f64`, which is `+0.0` for either signed zero.
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
