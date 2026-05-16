//! Top-level statement executor.

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
    /// Statement produced a binding table.
    Rows(BindingTable),
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
            txn.commit_with_principal(principal).map_err(|source| {
                ExecutorError::GraphMutation {
                    source,
                    span: SourceSpan::default(),
                }
            })?;
            Ok(output_from_table(plan, table))
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
