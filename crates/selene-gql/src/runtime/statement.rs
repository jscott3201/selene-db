//! Top-level statement executor.

use selene_core::Change;
use selene_graph::CommitOutcome;

use crate::{
    ExecutionPlan, ProcedureRegistry, SourceSpan, StatementCategory,
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
    match plan.category {
        StatementCategory::ReadOnly => execute_read_only(plan, session, registry),
        StatementCategory::DataModifying | StatementCategory::CatalogModifying => {
            execute_write(plan, session, registry)
        }
        StatementCategory::TransactionControl => execute_transaction_control(plan, session),
    }
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
