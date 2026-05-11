//! Opaque lifecycle principal identifier.

use selene_core::IStr;

/// Opaque caller identity attached to lifecycle events.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct Principal(IStr);

impl Principal {
    /// Construct a principal from an interned caller identity.
    #[must_use]
    pub const fn new(value: IStr) -> Self {
        Self(value)
    }

    /// Return the interned caller identity.
    #[must_use]
    pub const fn as_istr(self) -> IStr {
        self.0
    }
}
