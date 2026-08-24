//! Category-specific statement execution arms.
//!
//! Split out of `statement.rs` so the source-entry logic (caches, policies,
//! facade interception) and the per-category execution arms stay under the
//! file cap. Every function here is called from [`super::statement`] only.

use std::rc::Rc;

use selene_core::{CancellationToken, NodeScanBudget};
use selene_graph::CommitOutcome;

use super::session::materialize_parameter_values;
use crate::{
    ExecutionPlan, GqlStatus, ProcedureRegistry, SourceSpan,
    runtime::{
        BindingTable, BindingTableRegistry, ExecutorError, ExecutorWarning, Session,
        StatementOutput, TxContext, WriteOutcome, execute_plan, pipeline,
    },
};

pub(super) fn execute_read_only(
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

pub(super) fn execute_write(
    plan: &ExecutionPlan,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    if session.active_txn.is_some() {
        return execute_inside_explicit_tx(plan, session, registry);
    }
    execute_auto_commit(plan, session, registry)
}

pub(super) fn execute_maintenance(
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

pub(super) fn execute_transaction_control(
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

pub(super) fn execute_session_control(
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
