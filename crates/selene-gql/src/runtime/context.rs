//! Executor transaction context.

use std::{fmt, sync::Arc};

use selene_graph::{Mutator, SeleneGraph, WriteTxn};

use crate::{
    ProcedureRegistry, SourceSpan,
    plan::{ImplDefinedCaps, PipelineOpId},
    runtime::ExecutorError,
};

/// Adaptive re-optimization hook reserved for future executor phases.
pub trait AdaptiveOptimizer: Send + Sync {
    /// Observe output cardinality for one pipeline operation.
    fn observe_cardinality(&self, _op: PipelineOpId, _rows: u64) {}
}

/// Executor context for one statement.
///
/// Read-only contexts own an immutable graph snapshot so every scan in a
/// statement observes the same generation even if concurrent writers publish a
/// newer snapshot through [`selene_graph::SharedGraph`]. Write contexts also
/// carry a transaction-local working graph; reads then observe that working
/// graph so intra-statement writes are visible to later operators.
pub struct TxContext<'a, 'g> {
    snapshot: Arc<SeleneGraph>,
    impl_defined_caps: &'a ImplDefinedCaps,
    registry: &'a dyn ProcedureRegistry,
    reopt_hook: Option<&'a dyn AdaptiveOptimizer>,
    write_txn: Option<&'a mut WriteTxn<'g>>,
}

impl<'a, 'g> TxContext<'a, 'g> {
    /// Construct a read-only context over an immutable graph snapshot.
    #[must_use]
    pub fn read_only(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
    ) -> Self {
        Self {
            snapshot,
            impl_defined_caps,
            registry,
            reopt_hook: None,
            write_txn: None,
        }
    }

    /// Construct a read-only context carrying a future adaptive optimizer hook.
    #[must_use]
    pub fn read_only_with_reopt(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        reopt_hook: &'a dyn AdaptiveOptimizer,
    ) -> Self {
        Self {
            snapshot,
            impl_defined_caps,
            registry,
            reopt_hook: Some(reopt_hook),
            write_txn: None,
        }
    }

    /// Construct a write-capable context over a graph write transaction.
    #[must_use]
    pub fn write(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        txn: &'a mut WriteTxn<'g>,
    ) -> Self {
        Self {
            snapshot,
            impl_defined_caps,
            registry,
            reopt_hook: None,
            write_txn: Some(txn),
        }
    }

    /// Borrow the graph snapshot used by this context.
    ///
    /// Write contexts return the transaction-local working graph so later
    /// reads in the same statement see earlier writes.
    #[must_use]
    pub fn snapshot(&self) -> &SeleneGraph {
        if let Some(txn) = self.write_txn.as_deref() {
            return txn.read();
        }
        self.snapshot.as_ref()
    }

    /// Borrow the statement mutator.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::InvalidTransactionState`] when a mutation
    /// operator executes in a read-only context.
    pub fn mutator(&mut self) -> Result<Mutator<'_, 'g>, ExecutorError> {
        self.mutator_with_span(
            "mutation invoked without write transaction",
            SourceSpan::default(),
        )
    }

    /// Borrow the statement mutator with a caller-specific diagnostic.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::InvalidTransactionState`] when the current
    /// context is read-only.
    pub fn mutator_with_span(
        &mut self,
        detail: &'static str,
        span: SourceSpan,
    ) -> Result<Mutator<'_, 'g>, ExecutorError> {
        self.write_txn
            .as_deref_mut()
            .map(WriteTxn::mutator)
            .ok_or(ExecutorError::InvalidTransactionState { detail, span })
    }

    /// Confirm a write transaction is attached without borrowing its mutator.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::InvalidTransactionState`] when the current
    /// context is read-only.
    pub fn ensure_write_txn(
        &self,
        detail: &'static str,
        span: SourceSpan,
    ) -> Result<(), ExecutorError> {
        if self.write_txn.is_some() {
            Ok(())
        } else {
            Err(ExecutorError::InvalidTransactionState { detail, span })
        }
    }

    /// Borrow the planner/executor implementation-defined caps.
    #[must_use]
    pub const fn impl_defined_caps(&self) -> &'a ImplDefinedCaps {
        self.impl_defined_caps
    }

    /// Borrow the procedure registry for this statement.
    #[must_use]
    pub const fn registry(&self) -> &'a dyn ProcedureRegistry {
        self.registry
    }

    /// Borrow the adaptive optimizer hook, when one was supplied.
    #[must_use]
    pub const fn reopt_hook(&self) -> Option<&dyn AdaptiveOptimizer> {
        self.reopt_hook
    }
}

impl fmt::Debug for TxContext<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TxContext")
            .field("snapshot", &self.snapshot)
            .field("impl_defined_caps", self.impl_defined_caps)
            .field("reopt_hook", &self.reopt_hook.is_some())
            .field("write_txn", &self.write_txn.is_some())
            .finish()
    }
}
