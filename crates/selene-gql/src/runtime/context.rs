//! Executor transaction context.

use std::sync::Arc;

use selene_graph::SeleneGraph;

use crate::plan::ImplDefinedCaps;

/// Read-only executor context for one statement.
///
/// The context owns an immutable graph snapshot so every scan in a statement
/// observes the same generation even if concurrent writers publish a newer
/// snapshot through [`selene_graph::SharedGraph`].
#[derive(Clone, Debug)]
pub struct TxContext<'a> {
    snapshot: Arc<SeleneGraph>,
    impl_defined_caps: &'a ImplDefinedCaps,
}

impl<'a> TxContext<'a> {
    /// Construct a read-only context over an immutable graph snapshot.
    #[must_use]
    pub fn read_only(snapshot: Arc<SeleneGraph>, impl_defined_caps: &'a ImplDefinedCaps) -> Self {
        Self {
            snapshot,
            impl_defined_caps,
        }
    }

    /// Borrow the immutable graph snapshot used by this context.
    #[must_use]
    pub fn snapshot(&self) -> &SeleneGraph {
        self.snapshot.as_ref()
    }

    /// Borrow the planner/executor implementation-defined caps.
    #[must_use]
    pub const fn impl_defined_caps(&self) -> &ImplDefinedCaps {
        self.impl_defined_caps
    }
}
