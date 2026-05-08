//! Analyzer type cells.

use crate::GqlType;

/// Type carried by an analyzed expression cell.
///
/// `Dynamic` is the explicit sink for static-inference gaps. It is not a
/// hint downstream stages can ignore; planners and executors must handle it
/// deliberately until BRIEF-22 resolves more expressions statically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnalyzedType {
    /// Statically resolved type.
    Resolved(GqlType),
    /// Static inference did not resolve this expression in the current pass.
    Dynamic,
}

impl AnalyzedType {
    /// Canonical dynamic type cell used by BRIEF-21.
    pub const DYNAMIC: Self = Self::Dynamic;

    /// Return true when this type cell is dynamic.
    #[must_use]
    pub const fn is_dynamic(&self) -> bool {
        matches!(self, Self::Dynamic)
    }

    /// Return true when this type cell has a concrete static type.
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        matches!(self, Self::Resolved(_))
    }
}
