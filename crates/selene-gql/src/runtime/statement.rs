//! Top-level statement executor.

use std::{rc::Rc, sync::Arc, time::Instant};

use selene_core::{CancellationToken, Change, NodeScanBudget, metrics};
use selene_graph::CommitOutcome;

use super::plan_cache::{SharedPlanCacheInsert, SharedPlanCacheLookup};
use super::session::materialize_parameter_values;
use crate::{
    ExecutionPlan, GqlStatus, LiveIndexCatalog, OptimizeContext, PipelineOp, ProcedureRegistry,
    SourceSpan, StatementCategory, TxOp,
    analyze::analyze,
    ast::Statement,
    optimize,
    parser::parse,
    plan::plan_with_caps as build_plan,
    runtime::{
        BindingTable, BindingTableRegistry, CallPlanKey, ExecutorError, ExecutorWarning, Session,
        TxContext, execute_plan, pipeline,
    },
};

/// Result returned by statement-level execution.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StatementOutput {
    /// Statement completed without a row-bearing result.
    Empty,
    /// Statement published a graph generation.
    Written(WriteOutcome),
    /// Statement produced a binding table.
    Rows(BindingTable),
}

/// Metadata returned for a statement that committed graph changes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct WriteOutcome {
    /// Optional row result for write statements with `RETURN`.
    pub rows: Option<BindingTable>,
    /// Changes produced by the committed write transaction.
    pub changes: Vec<Change>,
    /// Published graph generation.
    pub generation: u64,
    /// Next node ID after the commit.
    pub next_node_id: u64,
    /// Next edge ID after the commit.
    pub next_edge_id: u64,
    /// Highest sequence reported by commit-critical durable providers.
    pub durable_at: Option<u64>,
}

impl WriteOutcome {
    pub(crate) fn from_commit(outcome: CommitOutcome, rows: Option<BindingTable>) -> Self {
        Self {
            rows,
            changes: outcome.changes,
            generation: outcome.generation,
            next_node_id: outcome.next_node_id,
            next_edge_id: outcome.next_edge_id,
            durable_at: outcome.durable_at,
        }
    }
}

/// Execute one planned statement against a caller-owned session.
///
/// The procedure registry argument is optional because statement kinds without
/// CALL should not force embedders to construct a registry.
#[tracing::instrument(
    name = "selene.gql.execute_statement",
    skip(plan, session, registry),
    fields(category = ?plan.category)
)]
pub fn execute_statement(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    // ISO/IEC 39075:2024 section 7.3: once `SESSION CLOSE` sets the termination
    // flag, every subsequent GQL-request is rejected regardless of category.
    // `execute_source` guards the source-string entry; this guard covers the
    // sibling public path where an embedder caches an `ExecutionPlan` and
    // re-executes it directly (the pipeline documented in the embedding guide),
    // so the termination flag is enforced at the single statement funnel
    // (hard rule 11) rather than only one layer up. The flag is set *during*
    // execution of `SESSION CLOSE` itself, so this check never blocks the
    // closing statement — only the requests that follow it.
    if session.is_closed() {
        return Err(ExecutorError::SessionClosed {
            span: SourceSpan::default(),
        });
    }
    if session.aborted && plan.category != StatementCategory::TransactionControl {
        return Err(ExecutorError::InFailedTransaction {
            span: SourceSpan::default(),
        });
    }
    let started = Instant::now();
    let counts_toward_tx =
        plan.category != StatementCategory::TransactionControl && session.active_txn.is_some();
    let result = match plan.category {
        StatementCategory::ReadOnly => execute_read_only(plan, session, registry),
        StatementCategory::Maintenance => execute_maintenance(plan, session, registry),
        StatementCategory::DataModifying | StatementCategory::CatalogModifying => {
            execute_write(plan, session, registry)
        }
        StatementCategory::TransactionControl => execute_transaction_control(plan, session),
        StatementCategory::SessionControl => execute_session_control(plan, session, registry),
    };
    if counts_toward_tx {
        if result.is_ok() {
            session.tx_statement_count = session.tx_statement_count.saturating_add(1);
        } else {
            session.aborted = true;
        }
    }
    record_statement_metrics(plan, started);
    result
}

impl Session<'_> {
    /// Parse, plan, and execute one source-string statement through this session.
    ///
    /// When the session has a plan cache, source strings whose cached plan was
    /// prepared against the current graph schema version skip parse, analyze,
    /// and plan. Short-lived sessions can additionally use the opt-in shared
    /// non-CALL source-plan cache, keyed by graph ID, schema version, registry
    /// version, source text, caps, and optimizer mode. Procedure-call-rooted
    /// statements use the separate opt-in shared CALL plan cache, keyed by
    /// graph ID, schema version, registry version, and formatter-canonical
    /// source. Embedded in-pipeline `CALL` remains uncached.
    /// If an active explicit transaction has uncommitted schema changes, both
    /// caches bypass lookup and insert so analysis sees transaction-local
    /// schema.
    pub fn execute_source(
        &mut self,
        source: &str,
        registry: &dyn ProcedureRegistry,
    ) -> Result<StatementOutput, ExecutorError> {
        // ISO/IEC 39075:2024 section 6 GR3 + section 7.3: once a session is
        // closed by `SESSION CLOSE`, every subsequent GQL-request is rejected.
        // This is the spec request boundary; embedders that drop the `Session`
        // struct still work, but the termination flag is the conformant model.
        if self.is_closed() {
            return Err(ExecutorError::SessionClosed {
                span: SourceSpan::default(),
            });
        }
        let schema_version = self.graph().schema_version();
        let registry_version = registry.registry_version();
        let top_level_call_candidate = is_top_level_call_candidate(source);
        let active_txn_has_schema_changes = self
            .active_txn
            .as_ref()
            .is_some_and(|txn| txn.has_schema_changes());
        if !active_txn_has_schema_changes
            && let Some(cached) = self
                .plan_cache
                .as_mut()
                .and_then(|cache| cache.get(source, schema_version))
        {
            return execute_statement(&cached, self, registry);
        }

        let shared_plan_cache = (!active_txn_has_schema_changes)
            .then(|| self.shared_plan_cache.as_ref().map(Arc::clone))
            .flatten();
        let call_plan_cache = (!active_txn_has_schema_changes)
            .then(|| self.call_plan_cache.as_ref().map(Arc::clone))
            .flatten();
        let cache_graph_id = if shared_plan_cache.is_some() || call_plan_cache.is_some() {
            Some(self.graph().read().graph_id())
        } else {
            None
        };
        if !top_level_call_candidate
            && let (Some(cache), Some(graph_id)) = (shared_plan_cache.as_ref(), cache_graph_id)
            && let Some(cached) = cache.get(SharedPlanCacheLookup {
                graph_id,
                schema_version,
                registry_version,
                source,
                caps: self.caps,
                index_selection: self.index_selection,
            })
        {
            if let Some(cache) = self.plan_cache.as_mut() {
                cache.insert(Arc::from(source), Arc::clone(&cached), schema_version);
            }
            return execute_statement(&cached, self, registry);
        }
        if top_level_call_candidate
            && let (Some(cache), Some(graph_id)) = (call_plan_cache.as_ref(), cache_graph_id)
            && let Some(cached) =
                cache.get_source(graph_id, schema_version, registry_version, source)
        {
            metrics::counter_inc(metrics::CALL_PLAN_CACHE_HITS_TOTAL);
            return execute_statement(&cached, self, registry);
        }

        // Codex PR #127 auto-review P2 #2 + P2 #3: compile failures inside an
        // explicit transaction must abort the transaction (PostgreSQL-style
        // contract: any error → rollback). Without this, parse/analyze/plan
        // errors would leave the explicit-tx in a usable state, diverging
        // from `execute_inside_explicit_tx` which marks `aborted = true` on
        // execution-time errors.
        let statement = parse(source).map_err(|source| {
            if self.active_txn.is_some() {
                self.aborted = true;
            }
            ExecutorError::Parse { source }
        })?;

        // Codex PR #127 auto-review P2 #1: short-circuit aborted-session
        // non-control statements before analyze/plan/cache-insert work.
        // TX-control statements (START/COMMIT/ROLLBACK) must remain reachable
        // on an aborted session so the caller can recover via ROLLBACK.
        if self.aborted && !is_tx_control_statement(&statement) {
            return Err(ExecutorError::InFailedTransaction {
                span: SourceSpan::default(),
            });
        }

        let call_plan_key = cache_graph_id.and_then(|graph_id| {
            CallPlanKey::for_statement(graph_id, schema_version, registry_version, &statement)
        });
        if let (Some(cache), Some(key)) = (call_plan_cache.as_ref(), call_plan_key.as_ref())
            && let Some(cached) = cache.get(key)
        {
            metrics::counter_inc(metrics::CALL_PLAN_CACHE_HITS_TOTAL);
            return execute_statement(&cached, self, registry);
        }

        let graph_type = self
            .active_txn
            .as_ref()
            .and_then(|txn| txn.read().meta.bound_type.as_ref().map(Arc::clone))
            .or_else(|| self.graph().graph_type());
        let analyzed = analyze(statement, registry, graph_type.as_deref()).map_err(|source| {
            if self.active_txn.is_some() {
                self.aborted = true;
            }
            ExecutorError::Analysis { source }
        })?;
        let lowered = build_plan(&analyzed, registry, &self.caps).map_err(|source| {
            if self.active_txn.is_some() {
                self.aborted = true;
            }
            ExecutorError::Plan { source }
        })?;
        // Optimize on the cache-MISS path only: cached plans are already
        // optimized, so the two cache-hit early-returns above serve optimized
        // plans at hit-cost. EXPLAIN renders the optimized inner plan for free
        // because the optimizer recurses into PipelineOp::ExplainPlan { inner }.
        let plan = Arc::new(self.optimize_plan(lowered));
        let source_arc = Arc::<str>::from(source);
        if !active_txn_has_schema_changes && let Some(cache) = self.plan_cache.as_mut() {
            cache.insert(Arc::clone(&source_arc), Arc::clone(&plan), schema_version);
        }
        if let (Some(cache), Some(graph_id)) = (shared_plan_cache, cache_graph_id) {
            cache.insert(
                SharedPlanCacheInsert {
                    graph_id,
                    schema_version,
                    registry_version,
                    source: Arc::clone(&source_arc),
                    caps: self.caps,
                    index_selection: self.index_selection,
                },
                Arc::clone(&plan),
            );
        }
        if let (Some(cache), Some(key)) = (call_plan_cache, call_plan_key) {
            cache.insert_with_source(key, source_arc, Arc::clone(&plan));
        }
        execute_statement(&plan, self, registry)
    }

    /// Run the default optimizer over a freshly-lowered plan.
    ///
    /// When [`index_selection`](Session::without_index_selection) is enabled
    /// (the default), the optimizer probes a snapshot-pinned
    /// [`LiveIndexCatalog`] so label / typed / composite index access paths are
    /// selected; Linear remains the always-correct fallback inside every rule.
    /// When disabled, the lowered (Linear) plan is returned unchanged — the
    /// optimizer fixed-point is skipped entirely, giving byte-identical Linear
    /// lowering and EXPLAIN to pre-optimizer-wiring HEAD.
    ///
    /// The catalog is built from a single pinned `Arc<SeleneGraph>` snapshot —
    /// the active transaction's working snapshot when inside an explicit
    /// transaction (so txn-local index DDL is visible), else the published
    /// snapshot. The plan-cache key is the `schema_version` epoch, which bumps
    /// only on schema-changing commits and is published after the new snapshot
    /// (see `selene_graph::WriteTxn::commit`); index *selection* depends only
    /// on which indexes exist, so a structural access path stays correct for
    /// any data mutation within an epoch.
    fn optimize_plan(&self, lowered: ExecutionPlan) -> ExecutionPlan {
        if !self.index_selection {
            return lowered;
        }
        let snapshot = match self.active_txn.as_ref() {
            Some(txn) => Arc::new(txn.read().clone()),
            None => self.graph().read(),
        };
        let catalog = LiveIndexCatalog::new(snapshot);
        let caps = lowered.impl_defined_caps;
        let ctx = OptimizeContext::new(&caps).with_index_catalog(&catalog);
        optimize(lowered, &ctx)
    }
}

fn is_tx_control_statement(statement: &Statement) -> bool {
    matches!(
        statement,
        Statement::StartTransaction { .. } | Statement::Commit { .. } | Statement::Rollback { .. }
    )
}

fn is_top_level_call_candidate(source: &str) -> bool {
    let source = source.trim_start();
    let Some(prefix) = source.get(..4) else {
        return false;
    };
    if !prefix.eq_ignore_ascii_case("CALL") {
        return false;
    }
    source[4..].chars().next().is_none_or(char::is_whitespace)
}

fn execute_read_only(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let providers = session.graph().index_providers();
    let snapshot = session.graph().read();
    let session_tz = session.effective_time_zone();
    let binding_tables = Rc::new(BindingTableRegistry::new());
    let parameters = materialize_parameter_values(
        &session.parameters,
        &session.scalar_parameters,
        &binding_tables,
    );
    let (cancellation, deadline, row_cap, node_scan_budget) = resource_limits(session);
    let warning_sink = session.warning_sink.as_ref();
    let table = if let Some(txn) = session.active_txn.as_mut() {
        let mut ctx = TxContext::write_with_owned_parameters_and_registry(
            snapshot,
            &plan.impl_defined_caps,
            registry,
            txn,
            providers,
            parameters,
            Rc::clone(&binding_tables),
        )
        .with_resource_limits(
            cancellation.as_ref(),
            deadline,
            row_cap,
            node_scan_budget.as_ref(),
        )
        .with_warning_sink(warning_sink)
        .with_session_time_zone(session_tz);
        ctx.check_cancellation()?;
        let table = execute_plan(plan, &mut ctx)?;
        note_output_rows(plan, &ctx, table.row_count())?;
        table
    } else {
        let mut ctx = TxContext::read_only_with_owned_parameters_and_registry(
            snapshot,
            &plan.impl_defined_caps,
            registry,
            providers,
            parameters,
            Rc::clone(&binding_tables),
        )
        .with_resource_limits(
            cancellation.as_ref(),
            deadline,
            row_cap,
            node_scan_budget.as_ref(),
        )
        .with_warning_sink(warning_sink)
        .with_session_time_zone(session_tz);
        ctx.check_cancellation()?;
        let table = execute_plan(plan, &mut ctx)?;
        note_output_rows(plan, &ctx, table.row_count())?;
        table
    };
    Ok(output_from_table(plan, table))
}

fn execute_write(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    if session.active_txn.is_some() {
        return execute_inside_explicit_tx(plan, session, registry);
    }
    execute_auto_commit(plan, session, registry)
}

fn execute_maintenance(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    if session.active_txn.is_some() {
        return Err(ExecutorError::InvalidTransactionState {
            detail: "maintenance procedure cannot run inside an explicit transaction",
            span: SourceSpan::default(),
        });
    }
    let providers = session.graph().index_providers();
    let snapshot = session.graph().read();
    let session_tz = session.effective_time_zone();
    let binding_tables = Rc::new(BindingTableRegistry::new());
    let parameters = materialize_parameter_values(
        &session.parameters,
        &session.scalar_parameters,
        &binding_tables,
    );
    let (cancellation, deadline, row_cap, node_scan_budget) = resource_limits(session);
    let warning_sink = session.warning_sink.as_ref();
    let mut ctx = TxContext::maintenance_with_owned_parameters_and_registry(
        snapshot,
        &plan.impl_defined_caps,
        registry,
        session.graph(),
        providers,
        parameters,
        Rc::clone(&binding_tables),
    )
    .with_resource_limits(
        cancellation.as_ref(),
        deadline,
        row_cap,
        node_scan_budget.as_ref(),
    )
    .with_warning_sink(warning_sink)
    .with_session_time_zone(session_tz);
    ctx.check_cancellation()?;
    let table = execute_plan(plan, &mut ctx)?;
    note_output_rows(plan, &ctx, table.row_count())?;
    Ok(output_from_table(plan, table))
}

fn execute_inside_explicit_tx(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let providers = session.graph().index_providers();
    let snapshot = session.graph().read();
    let session_tz = session.effective_time_zone();
    let binding_tables = Rc::new(BindingTableRegistry::new());
    let parameters = materialize_parameter_values(
        &session.parameters,
        &session.scalar_parameters,
        &binding_tables,
    );
    let (cancellation, deadline, row_cap, node_scan_budget) = resource_limits(session);
    let warning_sink = session.warning_sink.as_ref();
    let txn = session
        .active_txn
        .as_mut()
        .ok_or(ExecutorError::ImplementationDefined {
            detail: "explicit-TX path entered without active transaction",
        })?;
    let mut ctx = TxContext::write_with_owned_parameters_and_registry(
        snapshot,
        &plan.impl_defined_caps,
        registry,
        txn,
        providers,
        parameters,
        Rc::clone(&binding_tables),
    )
    .with_resource_limits(
        cancellation.as_ref(),
        deadline,
        row_cap,
        node_scan_budget.as_ref(),
    )
    .with_warning_sink(warning_sink)
    .with_session_time_zone(session_tz);
    let result = ctx
        .check_cancellation()
        .and_then(|()| execute_plan(plan, &mut ctx))
        .and_then(|table| {
            note_output_rows(plan, &ctx, table.row_count())?;
            Ok(table)
        });
    if result.is_err() {
        session.aborted = true;
    }
    result.map(|table| output_from_table(plan, table))
}

fn execute_auto_commit(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let providers = session.graph().index_providers();
    let snapshot = session.graph().read();
    let principal = session.principal();
    let session_tz = session.effective_time_zone();
    let binding_tables = Rc::new(BindingTableRegistry::new());
    let parameters = materialize_parameter_values(
        &session.parameters,
        &session.scalar_parameters,
        &binding_tables,
    );
    let mut txn = session.graph().begin_write();
    let (cancellation, deadline, row_cap, node_scan_budget) = resource_limits(session);
    let warning_sink = session.warning_sink.as_ref();
    let result = {
        let mut ctx = TxContext::write_with_owned_parameters_and_registry(
            snapshot,
            &plan.impl_defined_caps,
            registry,
            &mut txn,
            providers,
            parameters,
            Rc::clone(&binding_tables),
        )
        .with_resource_limits(
            cancellation.as_ref(),
            deadline,
            row_cap,
            node_scan_budget.as_ref(),
        )
        .with_warning_sink(warning_sink)
        .with_session_time_zone(session_tz);
        ctx.check_cancellation()
            .and_then(|()| execute_plan(plan, &mut ctx))
            .and_then(|table| {
                note_output_rows(plan, &ctx, table.row_count())?;
                Ok(table)
            })
    };
    match result {
        Ok(table) => {
            let outcome = txn.commit_with_principal(principal).map_err(|source| {
                ExecutorError::GraphMutation {
                    source,
                    span: SourceSpan::default(),
                }
            })?;
            emit_commit_warnings(&outcome, session);
            Ok(write_output_from_commit(plan, table, outcome))
        }
        Err(error) => {
            txn.rollback();
            Err(error)
        }
    }
}

fn emit_commit_warnings(outcome: &CommitOutcome, session: &Session<'_>) {
    let Some(sink) = session.warning_sink.as_ref() else {
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

fn note_output_rows(
    plan: &ExecutionPlan,
    ctx: &TxContext<'_, '_>,
    row_count: usize,
) -> Result<(), ExecutorError> {
    if !plan.output_schema.columns.is_empty() {
        ctx.note_result_rows(row_count)?;
    }
    Ok(())
}

fn resource_limits(
    session: &Session<'_>,
) -> (
    Option<CancellationToken>,
    Option<std::time::Instant>,
    Option<usize>,
    Option<NodeScanBudget>,
) {
    (
        session.cancellation.clone(),
        session.deadline,
        session.row_cap,
        session.max_nodes_scanned.map(NodeScanBudget::new),
    )
}

fn execute_transaction_control(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
) -> Result<StatementOutput, ExecutorError> {
    let [crate::PipelineOp::Tx(op)] = plan.pipeline.as_slice() else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "transaction-control plan must contain exactly one TX op",
        });
    };
    pipeline::tx::execute(op, session)
}

fn execute_session_control(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let [crate::PipelineOp::Session(op)] = plan.pipeline.as_slice() else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "session-control plan must contain exactly one session op",
        });
    };
    pipeline::session::execute(op, session, registry)
}

fn output_from_table(plan: &ExecutionPlan, table: BindingTable) -> StatementOutput {
    if plan.output_schema.columns.is_empty() {
        StatementOutput::Empty
    } else {
        StatementOutput::Rows(table)
    }
}

fn write_output_from_commit(
    plan: &ExecutionPlan,
    table: BindingTable,
    outcome: CommitOutcome,
) -> StatementOutput {
    let rows = if plan.output_schema.columns.is_empty() {
        None
    } else {
        Some(table)
    };
    StatementOutput::Written(WriteOutcome::from_commit(outcome, rows))
}

fn record_statement_metrics(plan: &ExecutionPlan, started: Instant) {
    let label = metrics::Label::new(metrics::STATEMENT_KIND_LABEL, statement_kind(plan));
    metrics::counter_inc_with_label(metrics::QUERIES_TOTAL, label);
    metrics::histogram_record_with_label(
        metrics::QUERY_DURATION_SECONDS,
        started.elapsed().as_secs_f64(),
        label,
    );
}

fn statement_kind(plan: &ExecutionPlan) -> &'static str {
    if let Some(kind) = plan.pipeline.iter().find_map(pipeline_statement_kind) {
        return kind;
    }
    match plan.category {
        StatementCategory::ReadOnly => "query",
        StatementCategory::DataModifying => "mutation",
        StatementCategory::CatalogModifying => "catalog",
        StatementCategory::Maintenance => "maintenance",
        StatementCategory::TransactionControl => "transaction",
        StatementCategory::SessionControl => "session",
    }
}

fn pipeline_statement_kind(op: &PipelineOp) -> Option<&'static str> {
    match op {
        PipelineOp::Union { .. } | PipelineOp::Chain(_) | PipelineOp::CorrelatedChain(_) => {
            Some("composite")
        }
        PipelineOp::Match(_) | PipelineOp::OptionalMatch(_) => Some("query"),
        PipelineOp::Call(_) => Some("call"),
        PipelineOp::CallSubquery(_) => Some("call_subquery"),
        PipelineOp::Mutation(_) => Some("mutation"),
        PipelineOp::Catalog(_) => Some("catalog"),
        PipelineOp::ExplainPlan { .. } => Some("explain"),
        PipelineOp::Tx(TxOp::Start { .. }) => Some("start_transaction"),
        PipelineOp::Tx(TxOp::Commit { .. }) => Some("commit"),
        PipelineOp::Tx(TxOp::Rollback { .. }) => Some("rollback"),
        _ => None,
    }
}
