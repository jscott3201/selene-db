//! Top-level statement executor.

use std::sync::Arc;

use selene_core::Change;
use selene_graph::CommitOutcome;

use crate::{
    ExecutionPlan, ProcedureRegistry, SourceSpan, StatementCategory,
    analyze::analyze,
    ast::Statement,
    parser::parse,
    plan::plan as build_plan,
    runtime::{BindingTable, ExecutorError, Session, TxContext, execute_plan, pipeline},
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
pub fn execute_statement(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    if session.aborted && plan.category != StatementCategory::TransactionControl {
        return Err(ExecutorError::InFailedTransaction {
            span: SourceSpan::default(),
        });
    }
    let counts_toward_tx =
        plan.category != StatementCategory::TransactionControl && session.active_txn.is_some();
    let result = match plan.category {
        StatementCategory::ReadOnly => execute_read_only(plan, session, registry),
        StatementCategory::DataModifying | StatementCategory::CatalogModifying => {
            execute_write(plan, session, registry)
        }
        StatementCategory::TransactionControl => execute_transaction_control(plan, session),
    };
    if counts_toward_tx && result.is_ok() {
        session.tx_statement_count = session.tx_statement_count.saturating_add(1);
    }
    result
}

impl Session<'_> {
    /// Parse, plan, and execute one source-string statement through this session.
    ///
    /// When the session has a plan cache, source strings whose cached plan was
    /// prepared against the current graph schema version skip parse, analyze,
    /// and plan. Plans containing procedure calls are not cached in v1.1 because
    /// procedure registries do not yet expose an invalidation epoch. If an
    /// active explicit transaction has uncommitted schema changes, the call
    /// bypasses lookup and insert so analysis sees the transaction-local schema.
    pub fn execute_source(
        &mut self,
        source: &str,
        registry: &dyn ProcedureRegistry,
    ) -> Result<StatementOutput, ExecutorError> {
        let schema_version = self.graph().schema_version();
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
        let plan = Arc::new(build_plan(&analyzed, registry).map_err(|source| {
            if self.active_txn.is_some() {
                self.aborted = true;
            }
            ExecutorError::Plan { source }
        })?);
        if !active_txn_has_schema_changes && let Some(cache) = self.plan_cache.as_mut() {
            cache.insert(Arc::from(source), Arc::clone(&plan), schema_version);
        }
        execute_statement(&plan, self, registry)
    }
}

fn is_tx_control_statement(statement: &Statement) -> bool {
    matches!(
        statement,
        Statement::StartTransaction { .. } | Statement::Commit { .. } | Statement::Rollback { .. }
    )
}

fn execute_read_only(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let providers = session.graph().index_providers();
    let snapshot = session.graph().read();
    let table = if let Some(txn) = session.active_txn.as_mut() {
        let mut ctx = TxContext::write(snapshot, &plan.impl_defined_caps, registry, txn, providers);
        execute_plan(plan, &mut ctx)?
    } else {
        let mut ctx = TxContext::read_only(snapshot, &plan.impl_defined_caps, registry, providers);
        execute_plan(plan, &mut ctx)?
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

fn execute_inside_explicit_tx(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let providers = session.graph().index_providers();
    let snapshot = session.graph().read();
    let txn = session
        .active_txn
        .as_mut()
        .ok_or(ExecutorError::ImplementationDefined {
            detail: "explicit-TX path entered without active transaction",
        })?;
    let mut ctx = TxContext::write(snapshot, &plan.impl_defined_caps, registry, txn, providers);
    let result = execute_plan(plan, &mut ctx);
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
    let mut txn = session.graph().begin_write();
    let result = {
        let mut ctx = TxContext::write(
            snapshot,
            &plan.impl_defined_caps,
            registry,
            &mut txn,
            providers,
        );
        execute_plan(plan, &mut ctx)
    };
    match result {
        Ok(table) => {
            let outcome = txn.commit_with_principal(principal).map_err(|source| {
                ExecutorError::GraphMutation {
                    source,
                    span: SourceSpan::default(),
                }
            })?;
            Ok(write_output_from_commit(plan, table, outcome))
        }
        Err(error) => {
            txn.rollback();
            Err(error)
        }
    }
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
