//! Allocation-free compatibility types used by generated data.

use std::fmt;

/// Stable feature or extension identifier used by runtime consumers.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct FeatureId(&'static str);

impl FeatureId {
    pub(crate) const fn new(value: &'static str) -> Self {
        Self(value)
    }

    /// Return the stable identifier text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for FeatureId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable implementation-defined identifier used by runtime consumers.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct AnnexBId(&'static str);

impl AnnexBId {
    pub(crate) const fn new(value: &'static str) -> Self {
        Self(value)
    }

    /// Return the implementation-defined identifier text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Existing runtime view of an implementation-defined choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImplDefinedChoice {
    /// Human-readable summary of the choice.
    pub choice: &'static str,
    /// Existing ownership citation for detailed behavior.
    pub settled_in: &'static str,
}
