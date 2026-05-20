//! Statement-session state for explicit transaction control.

use std::{cell::RefCell, collections::BTreeMap, num::NonZeroUsize, sync::Arc, time::Instant};

use selene_core::{CancellationToken, Change, IStr, IStrAdmissionPolicy, Value};
use selene_graph::{SharedGraph, WriteTxn};

use crate::{
    SourceSpan,
    runtime::{ExecutorError, PlanCache, PlanCacheStats, WarningSink, WriteOutcome},
};

/// Caller-owned executor session bound to one shared graph.
pub struct Session<'g> {
    graph: &'g SharedGraph,
    principal: Option<Arc<[u8]>>,
    pub(crate) parameters: BTreeMap<IStr, Value>,
    pub(crate) plan_cache: Option<PlanCache>,
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
    /// receives `01G11` when an aggregate eliminates NULL input values.
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
    pub fn bind_parameter(&mut self, name: IStr, value: Value) -> Option<Value> {
        self.parameters.insert(name, value)
    }

    /// Remove one session-local query parameter and return its prior value.
    pub fn clear_parameter(&mut self, name: &IStr) -> Option<Value> {
        self.parameters.remove(name)
    }

    /// Remove all session-local query parameters.
    pub fn clear_parameters(&mut self) {
        self.parameters.clear();
    }

    /// Borrow the session-local query-parameter map used for statement execution.
    #[must_use]
    pub(crate) fn parameters(&self) -> &BTreeMap<IStr, Value> {
        &self.parameters
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

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, thread, time::Instant};

    use selene_core::{GraphId, IStr, IStrAdmissionPolicy, intern_with_admission};
    use selene_graph::{GraphTypeDef, SharedGraph, TypedIndexKind};
    use selene_persist::{DEFAULT_WAL_FILE_NAME, WalConfig};

    use super::*;
    use crate::{
        analyze::analyze,
        parser::parse,
        plan::ExecutionPlan,
        plan::plan,
        procedure_registry::EmptyProcedureRegistry,
        runtime::statement::{StatementOutput, execute_statement},
    };

    fn planned(source: &str) -> ExecutionPlan {
        let statement = parse(source).expect("test input parses");
        let analyzed =
            analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
        plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
    }

    fn execute(source: &str, session: &mut Session<'_>) -> Result<StatementOutput, ExecutorError> {
        let plan = planned(source);
        execute_statement(&plan, session, &EmptyProcedureRegistry)
    }

    fn admitted(value: &str) -> IStr {
        intern_with_admission(value).expect("test name admits").0
    }

    fn empty_closed_graph(id: u64) -> SharedGraph {
        SharedGraph::builder(GraphId::new(id))
            .bound_to(GraphTypeDef {
                name: admitted("Empty"),
                node_types: Vec::new(),
                edge_types: Vec::new(),
            })
            .expect("empty type validates")
            .build()
            .expect("closed graph builds")
    }

    #[test]
    fn istr_admission_policy_is_session_scoped_across_threads() {
        let graph = SharedGraph::new(GraphId::new(3896));
        thread::scope(|scope| {
            let reject = scope.spawn(|| Session::new(&graph).istr_admission_policy);
            let fallback = scope.spawn(|| {
                Session::new(&graph)
                    .with_istr_admission_policy(IStrAdmissionPolicy::FallbackToExternal)
                    .istr_admission_policy
            });

            assert_eq!(
                reject.join().expect("reject session joins"),
                IStrAdmissionPolicy::Reject
            );
            assert_eq!(
                fallback.join().expect("fallback session joins"),
                IStrAdmissionPolicy::FallbackToExternal
            );
        });
    }

    #[test]
    fn session_without_cache_executes_source_normally() {
        let graph = SharedGraph::new(GraphId::new(3897));
        let mut session = Session::new(&graph);

        let output = session
            .execute_source("RETURN 1", &EmptyProcedureRegistry)
            .expect("source executes");

        let StatementOutput::Rows(table) = output else {
            panic!("RETURN should produce rows");
        };
        assert_eq!(table.row_count(), 1);
        assert!(session.plan_cache_stats().is_none());
    }

    #[test]
    fn session_with_cache_hits_on_second_source_execute() {
        let graph = SharedGraph::new(GraphId::new(3898));
        let mut session =
            Session::new(&graph).with_plan_cache(NonZeroUsize::new(4).expect("nonzero"));

        session
            .execute_source("RETURN 1", &EmptyProcedureRegistry)
            .expect("first source execute succeeds");
        session
            .execute_source("RETURN 1", &EmptyProcedureRegistry)
            .expect("second source execute succeeds");

        let stats = session.plan_cache_stats().expect("cache enabled");
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.stale_invalidations, 0);
    }

    #[test]
    fn session_schema_change_invalidates_cached_source_plan() {
        let graph = SharedGraph::new(GraphId::new(3899));
        let mut session =
            Session::new(&graph).with_plan_cache(NonZeroUsize::new(4).expect("nonzero"));

        session
            .execute_source("RETURN 1", &EmptyProcedureRegistry)
            .expect("first source execute succeeds");
        graph
            .create_property_index(admitted("Person"), admitted("age"), TypedIndexKind::I64)
            .expect("index creation bumps schema");
        session
            .execute_source("RETURN 1", &EmptyProcedureRegistry)
            .expect("second source execute succeeds");

        let stats = session.plan_cache_stats().expect("cache enabled");
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.stale_invalidations, 1);
    }

    #[test]
    fn execute_source_aborted_session_returns_in_failed_transaction_without_compiling() {
        // Codex PR #127 auto-review P2 #1: an aborted session must short-circuit
        // non-control source statements before analyze/plan/cache-insert work.
        let graph = SharedGraph::new(GraphId::new(3970));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");
        session
            .execute_source(
                "INSERT (n:Person) SET n.age = 1 / 0 FINISH",
                &EmptyProcedureRegistry,
            )
            .expect_err("division-by-zero aborts the transaction");
        assert!(session.is_aborted());

        let err = session
            .execute_source("RETURN 1", &EmptyProcedureRegistry)
            .expect_err("aborted session rejects non-control");
        assert!(matches!(err, ExecutorError::InFailedTransaction { .. }));
        session.abort();
    }

    #[test]
    fn execute_source_rollback_works_on_aborted_session() {
        // Aborted session must still accept TX-control statements (ROLLBACK
        // is the recovery path). Codex PR #127 auto-review P2 #1 contract.
        let graph = SharedGraph::new(GraphId::new(3971));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");
        session
            .execute_source(
                "INSERT (n:Person) SET n.age = 1 / 0 FINISH",
                &EmptyProcedureRegistry,
            )
            .expect_err("division-by-zero aborts the transaction");
        assert!(session.is_aborted());

        session
            .execute_source("ROLLBACK", &EmptyProcedureRegistry)
            .expect("rollback succeeds on aborted session");
        assert!(!session.is_aborted());
        assert!(!session.has_active_txn());
    }

    #[test]
    fn execute_source_parse_failure_aborts_active_tx() {
        // Codex PR #127 auto-review P2 #2: parse errors inside an explicit
        // transaction must abort the transaction (PostgreSQL-style "any error
        // → rollback" contract). Parallel for analyze/plan failures.
        let graph = SharedGraph::new(GraphId::new(3972));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");

        let err = session
            .execute_source("NOT A VALID GQL STATEMENT", &EmptyProcedureRegistry)
            .expect_err("malformed source errors");
        assert!(matches!(err, ExecutorError::Parse { .. }));
        assert!(session.is_aborted());
        session.abort();
    }

    #[test]
    fn execute_source_parse_failure_outside_tx_does_not_abort() {
        // Outside an explicit TX, the parse error returns as-is. Session has
        // no `aborted` semantics outside a TX (aborted flag is meaningless).
        let graph = SharedGraph::new(GraphId::new(3973));
        let mut session = Session::new(&graph);

        let err = session
            .execute_source("NOT A VALID GQL STATEMENT", &EmptyProcedureRegistry)
            .expect_err("malformed source errors");
        assert!(matches!(err, ExecutorError::Parse { .. }));
        assert!(!session.has_active_txn());
        assert!(!session.is_aborted());
    }

    #[test]
    fn execute_source_uses_transaction_local_schema_after_catalog_change() {
        let graph = empty_closed_graph(3901);
        let mut session =
            Session::new(&graph).with_plan_cache(NonZeroUsize::new(4).expect("nonzero"));

        session
            .execute_source("START TRANSACTION", &EmptyProcedureRegistry)
            .expect("start succeeds");
        session
            .execute_source("CREATE NODE TYPE :Person ()", &EmptyProcedureRegistry)
            .expect("catalog source succeeds");
        session
            .execute_source("INSERT (:Person)", &EmptyProcedureRegistry)
            .expect("insert sees transaction-local schema");
        session
            .execute_source("COMMIT", &EmptyProcedureRegistry)
            .expect("commit succeeds");

        assert_eq!(graph.read().node_count(), 1);
        assert_eq!(
            graph.graph_type().expect("closed graph").node_types[0]
                .name
                .as_str(),
            "Person"
        );
    }

    #[test]
    fn start_transaction_opens_active_txn() {
        let graph = SharedGraph::new(GraphId::new(3900));
        let mut session = Session::new(&graph);

        session.start_transaction().expect("start succeeds");

        assert!(session.has_active_txn());
        assert!(session.tx_started_at.is_some());
        assert_eq!(session.tx_statement_count, 0);
        session.abort();
    }

    #[test]
    fn start_transaction_nested_errors() {
        let graph = SharedGraph::new(GraphId::new(3901));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");

        let err = session
            .start_transaction()
            .expect_err("nested start errors");

        assert!(matches!(
            err,
            ExecutorError::TransactionAlreadyActive { .. }
        ));
        session.abort();
    }

    #[test]
    fn commit_transaction_no_active_errors() {
        let graph = SharedGraph::new(GraphId::new(3902));
        let mut session = Session::new(&graph);

        let err = session
            .commit_transaction()
            .expect_err("commit without transaction errors");

        assert!(matches!(err, ExecutorError::NoActiveTransaction { .. }));
    }

    #[test]
    fn rollback_transaction_no_active_errors() {
        let graph = SharedGraph::new(GraphId::new(3903));
        let mut session = Session::new(&graph);

        let err = session
            .rollback_transaction()
            .expect_err("rollback without transaction errors");

        assert!(matches!(err, ExecutorError::NoActiveTransaction { .. }));
    }

    #[test]
    fn commit_aggregates_changes_across_statements() {
        let graph = SharedGraph::new(GraphId::new(3904));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");

        execute("INSERT (:Person { name: 'a' })", &mut session).expect("first insert succeeds");
        execute("INSERT (:Person { name: 'b' })", &mut session).expect("second insert succeeds");
        execute("INSERT (:Person { name: 'c' })", &mut session).expect("third insert succeeds");
        let outcome = session.commit_transaction().expect("commit succeeds");

        assert_eq!(outcome.changes.len(), 3);
        assert_eq!(outcome.statement_count, 3);
        assert_eq!(graph.read().node_count(), 3);
    }

    #[test]
    fn commit_counts_read_only_statement_inside_transaction() {
        let graph = SharedGraph::new(GraphId::new(3905));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");

        execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
        execute("MATCH (n:Person) RETURN n", &mut session).expect("read succeeds");
        let outcome = session.commit_transaction().expect("commit succeeds");

        assert_eq!(outcome.changes.len(), 1);
        assert_eq!(outcome.statement_count, 2);
    }

    #[test]
    fn commit_returns_durable_at_with_core_provider() {
        let dir = tempfile::tempdir().expect("tempdir is created");
        let graph = SharedGraph::builder(GraphId::new(3906))
            .with_wal(dir.path().join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
            .expect("wal config opens")
            .build()
            .expect("graph builds");
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");

        execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
        let outcome = session.commit_transaction().expect("commit succeeds");

        assert_eq!(outcome.durable_at, Some(1));
    }

    #[test]
    fn commit_returns_no_durable_at_without_provider() {
        let graph = SharedGraph::new(GraphId::new(3907));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");

        execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
        let outcome = session.commit_transaction().expect("commit succeeds");

        assert_eq!(outcome.durable_at, None);
    }

    #[test]
    fn rollback_discards_changes() {
        let graph = SharedGraph::new(GraphId::new(3908));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");

        execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
        session.rollback_transaction().expect("rollback succeeds");

        assert_eq!(graph.read().node_count(), 0);
        assert!(!session.has_active_txn());
    }

    #[test]
    fn rollback_outcome_reports_count() {
        let graph = SharedGraph::new(GraphId::new(3909));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");

        execute("INSERT (:Person { name: 'a' })", &mut session).expect("first insert succeeds");
        execute("INSERT (:Person { name: 'b' })", &mut session).expect("second insert succeeds");
        let outcome = session.rollback_transaction().expect("rollback succeeds");

        assert_eq!(outcome.discarded_changes, 2);
        assert_eq!(outcome.statement_count, 2);
        assert_eq!(graph.read().node_count(), 0);
    }

    #[test]
    fn duration_micros_is_populated() {
        let graph = SharedGraph::new(GraphId::new(3910));
        let mut session = Session::new(&graph);
        let started = Instant::now();
        session.start_transaction().expect("start succeeds");

        execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
        let outcome = session.commit_transaction().expect("commit succeeds");

        assert!(outcome.duration_micros <= started.elapsed().as_micros() as u64);
    }

    #[test]
    fn failed_statement_does_not_increment_statement_count() {
        // Codex auto-review P2: tx_statement_count only counts ACCEPTED
        // non-control statements. A statement that errors inside the
        // transaction window must not bump the counter — rollback's
        // RollbackOutcome.statement_count must match docs.
        let graph = SharedGraph::new(GraphId::new(3912));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");

        execute("INSERT (:Person { name: 'a' })", &mut session).expect("first insert succeeds");
        execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
            .expect_err("division by zero aborts statement");
        let outcome = session.rollback_transaction().expect("rollback succeeds");

        // The first INSERT counts (1); the failed second statement does not.
        assert_eq!(outcome.statement_count, 1);
    }

    #[test]
    fn abort_clears_tx_state_fields() {
        let graph = SharedGraph::new(GraphId::new(3911));
        let mut session = Session::new(&graph);
        session.start_transaction().expect("start succeeds");
        execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");

        assert!(session.tx_started_at.is_some());
        assert_eq!(session.tx_statement_count, 1);
        session.abort();

        assert!(!session.has_active_txn());
        assert!(session.tx_started_at.is_none());
        assert_eq!(session.tx_statement_count, 0);
        assert_eq!(graph.read().node_count(), 0);
    }
}
