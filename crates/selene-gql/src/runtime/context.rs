//! Executor transaction context.

use std::{fmt, sync::Arc};

use selene_graph::SeleneGraph;

use crate::plan::{ImplDefinedCaps, PipelineOpId};

/// Adaptive re-optimization hook reserved for future executor phases.
pub trait AdaptiveOptimizer: Send + Sync {
    /// Observe output cardinality for one pipeline operation.
    fn observe_cardinality(&self, _op: PipelineOpId, _rows: u64) {}
}

/// Read-only executor context for one statement.
///
/// The context owns an immutable graph snapshot so every scan in a statement
/// observes the same generation even if concurrent writers publish a newer
/// snapshot through [`selene_graph::SharedGraph`].
#[derive(Clone)]
pub struct TxContext<'a> {
    snapshot: Arc<SeleneGraph>,
    impl_defined_caps: &'a ImplDefinedCaps,
    reopt_hook: Option<&'a dyn AdaptiveOptimizer>,
}

impl<'a> TxContext<'a> {
    /// Construct a read-only context over an immutable graph snapshot.
    #[must_use]
    pub fn read_only(snapshot: Arc<SeleneGraph>, impl_defined_caps: &'a ImplDefinedCaps) -> Self {
        Self {
            snapshot,
            impl_defined_caps,
            reopt_hook: None,
        }
    }

    /// Construct a read-only context carrying a future adaptive optimizer hook.
    #[must_use]
    pub fn read_only_with_reopt(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        reopt_hook: &'a dyn AdaptiveOptimizer,
    ) -> Self {
        Self {
            snapshot,
            impl_defined_caps,
            reopt_hook: Some(reopt_hook),
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

    /// Borrow the adaptive optimizer hook, when one was supplied.
    #[must_use]
    pub const fn reopt_hook(&self) -> Option<&dyn AdaptiveOptimizer> {
        self.reopt_hook
    }
}

impl fmt::Debug for TxContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TxContext")
            .field("snapshot", &self.snapshot)
            .field("impl_defined_caps", self.impl_defined_caps)
            .field("reopt_hook", &self.reopt_hook.is_some())
            .finish()
    }
}
