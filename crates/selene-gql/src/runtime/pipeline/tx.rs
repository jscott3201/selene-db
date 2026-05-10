//! Transaction-control statement executor.

use crate::{
    SourceSpan, TxOp,
    runtime::{ExecutorError, Session, StatementOutput},
};

/// Execute one top-level transaction-control operation.
pub(crate) fn execute(
    op: &TxOp,
    session: &mut Session<'_>,
) -> Result<StatementOutput, ExecutorError> {
    match op {
        TxOp::Start { span } => start(session, *span),
        TxOp::Commit { span } => commit(session, *span),
        TxOp::Rollback { span } => rollback(session, *span),
    }
}

fn start(session: &mut Session<'_>, span: SourceSpan) -> Result<StatementOutput, ExecutorError> {
    if session.active_txn.is_some() {
        return Err(ExecutorError::TransactionAlreadyActive { span });
    }
    session.active_txn = Some(session.graph().begin_write());
    Ok(StatementOutput::Empty)
}

fn commit(session: &mut Session<'_>, span: SourceSpan) -> Result<StatementOutput, ExecutorError> {
    let txn = session
        .active_txn
        .take()
        .ok_or(ExecutorError::NoActiveTransaction { span })?;
    txn.commit_with_principal(session.principal())
        .map_err(|source| ExecutorError::GraphMutation { source, span })?;
    Ok(StatementOutput::Empty)
}

fn rollback(session: &mut Session<'_>, span: SourceSpan) -> Result<StatementOutput, ExecutorError> {
    let txn = session
        .active_txn
        .take()
        .ok_or(ExecutorError::NoActiveTransaction { span })?;
    txn.rollback();
    Ok(StatementOutput::Empty)
}
