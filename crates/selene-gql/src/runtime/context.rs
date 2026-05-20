//! Executor transaction context.

use std::{cell::Cell, collections::BTreeMap, fmt, sync::Arc, time::Instant};

use selene_core::{CancellationCause, CancellationChecker, CancellationToken, IStr, Value};
use selene_graph::{IndexProvider, Mutator, SeleneGraph, WriteTxn};

use crate::{
    ProcedureRegistry, SourceSpan,
    analyze::ExprIdLookup,
    plan::SubqueryRegistry,
    plan::{ImplDefinedCaps, PipelineOpId},
    runtime::ExecutorError,
};

/// Adaptive re-optimization hook reserved for future executor phases.
pub trait AdaptiveOptimizer: Send + Sync {
    /// Observe output cardinality for one pipeline operation.
    fn observe_cardinality(&self, _op: PipelineOpId, _rows: u64) {}
}

static EMPTY_PARAMETERS: BTreeMap<IStr, Value> = BTreeMap::new();

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
    providers: &'a [Arc<dyn IndexProvider>],
    parameters: &'a BTreeMap<IStr, Value>,
    reopt_hook: Option<&'a dyn AdaptiveOptimizer>,
    plan_expr_ids: Option<&'a ExprIdLookup>,
    plan_subqueries: Option<&'a SubqueryRegistry>,
    cancellation: Option<&'a CancellationToken>,
    deadline: Option<Instant>,
    row_cap: Option<usize>,
    result_rows_emitted: Cell<usize>,
    write_txn: Option<&'a mut WriteTxn<'g>>,
}

/// Expression-evaluation context for one planned execution point.
///
/// Expression subqueries are planned into side tables on the execution plan.
/// The evaluator borrows those side tables through this wrapper while all
/// graph, parameter, and procedure access continues to flow through
/// [`TxContext`].
pub struct EvalCtx<'a, 'ctx, 'g, 'plan> {
    /// Transaction context for graph and parameter access.
    pub tx: &'a TxContext<'ctx, 'g>,
    /// Plan-owned expression IDs cloned from analyzer output.
    pub expr_ids: &'plan ExprIdLookup,
    /// Plan-owned expression-subquery registry.
    pub subqueries: &'plan SubqueryRegistry,
}

impl<'a, 'ctx, 'g, 'plan> EvalCtx<'a, 'ctx, 'g, 'plan> {
    /// Borrow the same transaction context with a different plan registry.
    #[must_use]
    pub const fn with_plan<'next>(
        &self,
        expr_ids: &'next ExprIdLookup,
        subqueries: &'next SubqueryRegistry,
    ) -> EvalCtx<'a, 'ctx, 'g, 'next> {
        EvalCtx {
            tx: self.tx,
            expr_ids,
            subqueries,
        }
    }
}

impl<'a, 'g> TxContext<'a, 'g> {
    /// Construct a read-only context over an immutable graph snapshot.
    #[must_use]
    pub fn read_only(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
    ) -> Self {
        Self::read_only_with_parameters(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            &EMPTY_PARAMETERS,
        )
    }

    pub(crate) fn read_only_with_parameters(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
        parameters: &'a BTreeMap<IStr, Value>,
    ) -> Self {
        Self {
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            parameters,
            reopt_hook: None,
            plan_expr_ids: None,
            plan_subqueries: None,
            cancellation: None,
            deadline: None,
            row_cap: None,
            result_rows_emitted: Cell::new(0),
            write_txn: None,
        }
    }

    /// Construct a read-only context carrying a future adaptive optimizer hook.
    #[must_use]
    pub fn read_only_with_reopt(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
        reopt_hook: &'a dyn AdaptiveOptimizer,
    ) -> Self {
        Self::read_only_with_parameters_and_reopt(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            reopt_hook,
            &EMPTY_PARAMETERS,
        )
    }

    pub(crate) fn read_only_with_parameters_and_reopt(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
        reopt_hook: &'a dyn AdaptiveOptimizer,
        parameters: &'a BTreeMap<IStr, Value>,
    ) -> Self {
        Self {
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            parameters,
            reopt_hook: Some(reopt_hook),
            plan_expr_ids: None,
            plan_subqueries: None,
            cancellation: None,
            deadline: None,
            row_cap: None,
            result_rows_emitted: Cell::new(0),
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
        providers: &'a [Arc<dyn IndexProvider>],
    ) -> Self {
        Self::write_with_parameters(
            snapshot,
            impl_defined_caps,
            registry,
            txn,
            providers,
            &EMPTY_PARAMETERS,
        )
    }

    pub(crate) fn write_with_parameters(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        txn: &'a mut WriteTxn<'g>,
        providers: &'a [Arc<dyn IndexProvider>],
        parameters: &'a BTreeMap<IStr, Value>,
    ) -> Self {
        Self {
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            parameters,
            reopt_hook: None,
            plan_expr_ids: None,
            plan_subqueries: None,
            cancellation: None,
            deadline: None,
            row_cap: None,
            result_rows_emitted: Cell::new(0),
            write_txn: Some(txn),
        }
    }

    /// Attach per-statement cooperative cancellation and output row-cap limits.
    #[must_use]
    pub fn with_resource_limits(
        mut self,
        cancellation: Option<&'a CancellationToken>,
        deadline: Option<Instant>,
        row_cap: Option<usize>,
    ) -> Self {
        self.cancellation = cancellation;
        self.deadline = deadline;
        self.row_cap = row_cap;
        self
    }

    /// Check the token and deadline at a cooperative cancellation point.
    pub(crate) fn check_cancellation(&self) -> Result<(), ExecutorError> {
        self.cancellation_checker()
            .check()
            .map_err(|cause| self.cancellation_error(cause, SourceSpan::default()))
    }

    /// Count outermost result rows and enforce the optional row cap.
    pub(crate) fn note_result_rows(&self, n: usize) -> Result<(), ExecutorError> {
        let Some(cap) = self.row_cap else {
            return Ok(());
        };
        let Some(next) = self.result_rows_emitted.get().checked_add(n) else {
            return Err(ExecutorError::RowCapExceeded {
                cap,
                span: SourceSpan::default(),
            });
        };
        self.result_rows_emitted.set(next);
        if next > cap {
            return Err(ExecutorError::RowCapExceeded {
                cap,
                span: SourceSpan::default(),
            });
        }
        Ok(())
    }

    /// Build a checker that can cross into procedure packs and algorithm crates.
    #[must_use]
    pub(crate) const fn cancellation_checker(&self) -> CancellationChecker<'_> {
        CancellationChecker::new(self.cancellation, self.deadline)
    }

    pub(crate) fn cancellation_error(
        &self,
        cause: CancellationCause,
        span: SourceSpan,
    ) -> ExecutorError {
        match cause {
            CancellationCause::Cancelled => ExecutorError::Cancelled { span },
            CancellationCause::Timeout { elapsed } => ExecutorError::Timeout {
                deadline: self.deadline.unwrap_or_else(Instant::now),
                elapsed,
                span,
            },
        }
    }

    /// Attach plan-owned expression metadata for direct pipeline execution.
    ///
    /// Top-level statement execution passes this metadata directly from the
    /// owning [`crate::ExecutionPlan`]. Embedding and test-harness code that
    /// runs `execute_pipeline(&plan.pipeline, ...)` directly should attach the
    /// same side tables so planned expression subqueries can be evaluated.
    #[must_use]
    pub fn with_plan_metadata(
        mut self,
        expr_ids: &'a ExprIdLookup,
        subqueries: &'a SubqueryRegistry,
    ) -> Self {
        self.plan_expr_ids = Some(expr_ids);
        self.plan_subqueries = Some(subqueries);
        self
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

    /// Borrow the fixed index-provider registry visible to this statement.
    #[must_use]
    pub const fn providers(&self) -> &'a [Arc<dyn IndexProvider>] {
        self.providers
    }

    /// Borrow the session-local query parameters visible to this statement.
    #[must_use]
    pub const fn parameters(&self) -> &'a BTreeMap<IStr, Value> {
        self.parameters
    }

    /// Borrow the adaptive optimizer hook, when one was supplied.
    #[must_use]
    pub const fn reopt_hook(&self) -> Option<&dyn AdaptiveOptimizer> {
        self.reopt_hook
    }

    pub(crate) const fn plan_metadata(&self) -> Option<(&'a ExprIdLookup, &'a SubqueryRegistry)> {
        match (self.plan_expr_ids, self.plan_subqueries) {
            (Some(expr_ids), Some(subqueries)) => Some((expr_ids, subqueries)),
            _ => None,
        }
    }
}

impl fmt::Debug for TxContext<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TxContext")
            .field("snapshot", &self.snapshot)
            .field("impl_defined_caps", self.impl_defined_caps)
            .field("providers", &self.providers.len())
            .field("parameters", &self.parameters.len())
            .field("reopt_hook", &self.reopt_hook.is_some())
            .field("plan_expr_ids", &self.plan_expr_ids.is_some())
            .field("plan_subqueries", &self.plan_subqueries.is_some())
            .field("cancellation", &self.cancellation.is_some())
            .field("deadline", &self.deadline.is_some())
            .field("row_cap", &self.row_cap)
            .field("result_rows_emitted", &self.result_rows_emitted.get())
            .field("write_txn", &self.write_txn.is_some())
            .finish()
    }
}
