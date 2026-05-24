//! Statement-session state for explicit transaction control.

use std::{cell::RefCell, collections::BTreeMap, num::NonZeroUsize, sync::Arc, time::Instant};

use selene_core::{CancellationToken, Change, IStr, IStrAdmissionPolicy, Value};
use selene_graph::{CommitOutcome, SharedGraph, WriteTxn};

use crate::{
    GqlStatus, SourceSpan,
    runtime::{
        BindingTable, BindingTableRegistry, CallPlanCache, ExecutorError, ExecutorWarning,
        PlanCache, PlanCacheStats, WarningSink, WriteOutcome,
    },
};

/// Session-local query parameter value.
///
/// Scalars are copied into each statement's parameter map directly. Table
/// parameters are registered in that statement's binding-table registry and
/// materialized as request-scoped `TableRef` values.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionParameterValue {
    /// Scalar query parameter value.
    Scalar(Value),
    /// Binding-table query parameter value.
    Table(Arc<BindingTable>),
}

/// Caller-owned executor session bound to one shared graph.
pub struct Session<'g> {
    graph: &'g SharedGraph,
    principal: Option<Arc<[u8]>>,
    pub(crate) parameters: BTreeMap<IStr, SessionParameterValue>,
    pub(crate) plan_cache: Option<PlanCache>,
    pub(crate) call_plan_cache: Option<Arc<CallPlanCache>>,
    pub(crate) active_txn: Option<WriteTxn<'g>>,
    pub(crate) aborted: bool,
    pub(crate) tx_started_at: Option<Instant>,
    pub(crate) tx_statement_count: u32,
    pub(crate) cancellation: Option<CancellationToken>,
    pub(crate) deadline: Option<Instant>,
    pub(crate) row_cap: Option<usize>,
    pub(crate) istr_admission_policy: IStrAdmissionPolicy,
    pub(crate) warning_sink: Option<RefCell<Box<dyn WarningSink>>>,
}

/// Metadata returned after committing an explicit transaction through a [`Session`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TransactionOutcome {
    /// Total changes aggregated across all statements in the transaction.
    pub changes: Vec<Change>,
    /// Graph generation published by the commit.
    pub generation: u64,
    /// Next node ID after the commit.
    pub next_node_id: u64,
    /// Next edge ID after the commit.
    pub next_edge_id: u64,
    /// Highest sequence reported by commit-critical durable providers.
    pub durable_at: Option<u64>,
    /// Wall-clock duration from `start_transaction` to commit completion.
    pub duration_micros: u64,
    /// Number of accepted non-control statements in the transaction window.
    pub statement_count: u32,
}

impl TransactionOutcome {
    pub(crate) fn into_write_outcome(self) -> WriteOutcome {
        WriteOutcome {
            rows: None,
            changes: self.changes,
            generation: self.generation,
            next_node_id: self.next_node_id,
            next_edge_id: self.next_edge_id,
            durable_at: self.durable_at,
        }
    }
}

/// Metadata returned after rolling back an explicit transaction through a [`Session`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RollbackOutcome {
    /// Count of changes discarded by the rollback.
    pub discarded_changes: usize,
    /// Number of accepted non-control statements in the transaction window.
    pub statement_count: u32,
    /// Wall-clock duration from `start_transaction` to rollback completion.
    pub duration_micros: u64,
}

impl<'g> Session<'g> {
    /// Create a session without commit-principal bytes.
    #[must_use]
    pub const fn new(graph: &'g SharedGraph) -> Self {
        Self {
            graph,
            principal: None,
            parameters: BTreeMap::new(),
            plan_cache: None,
            call_plan_cache: None,
            active_txn: None,
            aborted: false,
            tx_started_at: None,
            tx_statement_count: 0,
            cancellation: None,
            deadline: None,
            row_cap: None,
            istr_admission_policy: IStrAdmissionPolicy::Reject,
            warning_sink: None,
        }
    }

    /// Create a session that forwards opaque principal bytes to commits.
    #[must_use]
    pub fn with_principal(graph: &'g SharedGraph, principal: Arc<[u8]>) -> Self {
        Self {
            graph,
            principal: Some(principal),
            parameters: BTreeMap::new(),
            plan_cache: None,
            call_plan_cache: None,
            active_txn: None,
            aborted: false,
            tx_started_at: None,
            tx_statement_count: 0,
            cancellation: None,
            deadline: None,
            row_cap: None,
            istr_admission_policy: IStrAdmissionPolicy::Reject,
            warning_sink: None,
        }
    }

    /// Attach a cooperative cancellation token to subsequent statements.
    ///
    /// Cancellation is cooperative: statements observe the token at executor,
    /// procedure-pack, and algorithm checkpoints. If a statement inside an
    /// explicit transaction returns `Cancelled`, the transaction enters the
    /// failed state until `ROLLBACK`.
    #[must_use]
    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancellation = Some(token);
        self
    }

    /// Attach an absolute per-statement deadline to subsequent statements.
    ///
    /// The deadline is compared with `Instant::now()` at the same cooperative
    /// checkpoints as cancellation. Expiry returns `Timeout`; inside an
    /// explicit transaction that also marks the transaction failed until
    /// `ROLLBACK`.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Attach an outermost result-row cap to subsequent statements.
    ///
    /// The cap is enforced only at the statement output boundary. Intermediate
    /// rows produced by scans, joins, `UNWIND`, or other pipeline operators do
    /// not count against it. Exceeding the cap returns `RowCapExceeded`; inside
    /// an explicit transaction that marks the transaction failed until
    /// `ROLLBACK`.
    #[must_use]
    pub fn with_row_cap(mut self, max_rows: usize) -> Self {
        self.row_cap = Some(max_rows);
        self
    }

    /// Set the policy used when engine-created strings cross admission boundaries.
    ///
    /// The default is [`IStrAdmissionPolicy::Reject`], preserving the hard-error
    /// behavior of v1.0. [`IStrAdmissionPolicy::FallbackToExternal`] lets
    /// eligible runtime boundaries carry over-cap strings as
    /// [`Value::ExternalString`].
    #[must_use]
    pub fn with_istr_admission_policy(mut self, policy: IStrAdmissionPolicy) -> Self {
        self.istr_admission_policy = policy;
        self
    }

    /// Attach an opt-in runtime warning sink to subsequent statements.
    ///
    /// Sessions without a sink silently discard warnings. The sink currently
    /// receives ISO warning records such as `01G11` for aggregate NULL
    /// elimination and `01N01` for relaxed validation-mode writes; see
    /// `docs/embedding-guide.md` for an embedder-side collection example.
    #[must_use]
    pub fn with_warning_sink(mut self, sink: impl WarningSink + 'static) -> Self {
        self.warning_sink = Some(RefCell::new(Box::new(sink)));
        self
    }

    /// Bind or replace a session-local query parameter.
    ///
    /// Parameters are named without the leading `$` and are resolved by
    /// `$name` references during statement execution. Binding is an upsert:
    /// rebinding a name replaces the prior value and affects subsequent
    /// statements only. Parameters are session-level metadata, so transaction
    /// boundaries and [`Self::abort`] preserve the map. Parameters not
    /// referenced by a statement are ignored. Session plan-cache keys remain
    /// source-only; parameter values and runtime types are checked during each
    /// execution.
    ///
    /// Runtime positions that require a specific type validate strictly; for
    /// example, `LIMIT $n` accepts only non-negative integer values and returns
    /// [`ExecutorError::InvalidParameterType`] for mismatches.
    ///
    /// If `name` previously held a table binding, the table is replaced and
    /// `None` is returned. Use [`Self::bind_table_parameter`] when callers need
    /// table-aware replacement information.
    pub fn bind_parameter(&mut self, name: IStr, value: Value) -> Option<Value> {
        match self
            .parameters
            .insert(name, SessionParameterValue::Scalar(value))
        {
            Some(SessionParameterValue::Scalar(prior)) => Some(prior),
            Some(SessionParameterValue::Table(_)) | None => None,
        }
    }

    /// Bind or replace a session-local query parameter with a binding table.
    ///
    /// The table is stored at session scope and materialized into a fresh
    /// request-scoped table reference for each statement execution.
    pub fn bind_table_parameter(
        &mut self,
        name: IStr,
        table: BindingTable,
    ) -> Option<SessionParameterValue> {
        self.parameters
            .insert(name, SessionParameterValue::Table(Arc::new(table)))
    }

    /// Remove one session-local query parameter and return its prior scalar value.
    ///
    /// If `name` held a table binding, the table is removed and `None` is
    /// returned.
    pub fn clear_parameter(&mut self, name: &IStr) -> Option<Value> {
        match self.parameters.remove(name) {
            Some(SessionParameterValue::Scalar(prior)) => Some(prior),
            Some(SessionParameterValue::Table(_)) | None => None,
        }
    }

    /// Remove all session-local query parameters.
    pub fn clear_parameters(&mut self) {
        self.parameters.clear();
    }

    /// Borrow the session-local query-parameter map used for statement execution.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn parameters(&self) -> &BTreeMap<IStr, SessionParameterValue> {
        &self.parameters
    }

    pub(crate) fn materialize_parameters(
        &self,
        registry: &BindingTableRegistry,
    ) -> BTreeMap<IStr, Value> {
        self.parameters
            .iter()
            .map(|(name, value)| {
                let value = match value {
                    SessionParameterValue::Scalar(value) => value.clone(),
                    SessionParameterValue::Table(table) => {
                        Value::TableRef(registry.register(Arc::clone(table)))
                    }
                };
                (*name, value)
            })
            .collect()
    }

    /// Enable this session's source-string plan cache with the given capacity.
    ///
    /// The cache is Session-local and invalidates entries when the backing
    /// graph's schema-version epoch changes.
    #[must_use]
    pub fn with_plan_cache(mut self, capacity: NonZeroUsize) -> Self {
        self.plan_cache = Some(PlanCache::new(capacity));
        self
    }

    /// Enable this session's shared procedure-CALL plan cache.
    ///
    /// Embedders should pass one shared cache per graph so short-lived
    /// sessions can reuse procedure-call plans across requests. The cache key
    /// includes the graph ID, schema-version epoch, and procedure-registry
    /// version.
    #[must_use]
    pub fn with_call_plan_cache(mut self, cache: Arc<CallPlanCache>) -> Self {
        self.call_plan_cache = Some(cache);
        self
    }

    /// Return this session's plan-cache counters, if caching is enabled.
    #[must_use]
    pub fn plan_cache_stats(&self) -> Option<PlanCacheStats> {
        self.plan_cache.as_ref().map(PlanCache::stats)
    }

    /// Clear this session's cached plans without resetting counters.
    pub fn clear_plan_cache(&mut self) {
        if let Some(cache) = self.plan_cache.as_mut() {
            cache.clear();
        }
    }

    /// Borrow the graph this session executes against.
    #[must_use]
    pub(crate) const fn graph(&self) -> &'g SharedGraph {
        self.graph
    }

    /// Clone the principal bytes for a commit boundary.
    #[must_use]
    pub(crate) fn principal(&self) -> Option<Arc<[u8]>> {
        self.principal.clone()
    }

    /// Return true when the session owns an explicit write transaction.
    #[must_use]
    pub const fn has_active_txn(&self) -> bool {
        self.active_txn.is_some()
    }

    /// Return true when the active explicit transaction is aborted.
    #[must_use]
    pub const fn is_aborted(&self) -> bool {
        self.aborted
    }

    /// Open an explicit write transaction.
    ///
    /// Subsequent non-control statements executed through this session run
    /// inside the transaction until [`Self::commit_transaction`] or
    /// [`Self::rollback_transaction`] closes it.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::TransactionAlreadyActive`] when this session
    /// already owns an explicit transaction.
    pub fn start_transaction(&mut self) -> Result<(), ExecutorError> {
        if self.active_txn.is_some() {
            return Err(ExecutorError::TransactionAlreadyActive {
                span: SourceSpan::default(),
            });
        }
        self.active_txn = Some(self.graph.begin_write());
        self.tx_started_at = Some(Instant::now());
        self.tx_statement_count = 0;
        self.aborted = false;
        Ok(())
    }

    /// Commit the open explicit transaction.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::NoActiveTransaction`] when no explicit
    /// transaction is open, [`ExecutorError::InFailedTransaction`] when the
    /// transaction has been aborted by a failed statement, or
    /// [`ExecutorError::GraphMutation`] when the graph commit is rejected.
    pub fn commit_transaction(&mut self) -> Result<TransactionOutcome, ExecutorError> {
        if self.aborted {
            if let Some(txn) = self.active_txn.take() {
                txn.rollback();
            }
            self.clear_tx_state();
            return Err(ExecutorError::InFailedTransaction {
                span: SourceSpan::default(),
            });
        }
        let txn = self
            .active_txn
            .take()
            .ok_or(ExecutorError::NoActiveTransaction {
                span: SourceSpan::default(),
            })?;
        let statement_count = self.tx_statement_count;
        let outcome = txn.commit_with_principal(self.principal.clone());
        let duration_micros = self.tx_duration_micros();
        self.clear_tx_state();
        let outcome = outcome.map_err(|source| ExecutorError::GraphMutation {
            source,
            span: SourceSpan::default(),
        })?;
        emit_commit_warnings(&outcome, self.warning_sink.as_ref());
        Ok(TransactionOutcome {
            changes: outcome.changes,
            generation: outcome.generation,
            next_node_id: outcome.next_node_id,
            next_edge_id: outcome.next_edge_id,
            durable_at: outcome.durable_at,
            duration_micros,
            statement_count,
        })
    }

    /// Roll back the open explicit transaction.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::NoActiveTransaction`] when no explicit
    /// transaction is open.
    pub fn rollback_transaction(&mut self) -> Result<RollbackOutcome, ExecutorError> {
        let txn = self
            .active_txn
            .take()
            .ok_or(ExecutorError::NoActiveTransaction {
                span: SourceSpan::default(),
            })?;
        let discarded_changes = txn.change_count();
        let statement_count = self.tx_statement_count;
        let duration_micros = self.tx_duration_micros();
        txn.rollback();
        self.clear_tx_state();
        Ok(RollbackOutcome {
            discarded_changes,
            statement_count,
            duration_micros,
        })
    }

    /// Flush every commit-critical durable provider registered on this graph.
    ///
    /// Returns the highest durable sequence reported by providers, or `None`
    /// when the graph has no durable providers.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::Flush`] when any provider-owned flush fails.
    pub fn flush(&self) -> Result<Option<u64>, ExecutorError> {
        let mut highest = None;
        for provider in self.graph.durable_providers() {
            let tag = provider.provider_tag();
            let seq = provider.flush().map_err(|error| ExecutorError::Flush {
                provider_tag: tag,
                reason: error.to_string(),
            })?;
            if let Some(seq) = seq {
                highest = Some(highest.map_or(seq, |current: u64| current.max(seq)));
            }
        }
        Ok(highest)
    }

    /// Roll back and clear the explicit transaction, when one is active.
    pub fn abort(&mut self) {
        if let Some(txn) = self.active_txn.take() {
            txn.rollback();
        }
        self.clear_tx_state();
    }

    fn tx_duration_micros(&self) -> u64 {
        self.tx_started_at
            .map_or(0, |started| started.elapsed().as_micros() as u64)
    }

    fn clear_tx_state(&mut self) {
        self.aborted = false;
        self.tx_started_at = None;
        self.tx_statement_count = 0;
    }
}

fn emit_commit_warnings(
    outcome: &CommitOutcome,
    warning_sink: Option<&RefCell<Box<dyn WarningSink>>>,
) {
    let Some(sink) = warning_sink else {
        return;
    };
    for warning in &outcome.warnings {
        sink.borrow_mut().emit(ExecutorWarning {
            code: GqlStatus::VALIDATION_MODE_RELAXED_WRITE,
            message: warning.warning.violation.to_string(),
            span: SourceSpan::default(),
        });
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod session_tests;
