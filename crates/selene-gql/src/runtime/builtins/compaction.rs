//! `selene.compaction_stats` and `selene.compact` native built-ins.
//!
//! Compaction is engine maintenance: deletes already clear heavyweight row
//! payloads, while compaction later densifies row storage. The stats procedure
//! is read-only graph-tier observability; the compact procedure is
//! maintenance-tier row reclamation through [`MaintenanceContext`].

use selene_core::{CancellationCause, Value};
use selene_graph::CompactionStats;

use super::meta::{StaticOutputColumn, StaticParameter};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, MaintenanceContext, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const STATS_PROC_NAME: &str = "selene.compaction_stats";
const COMPACT_PROC_NAME: &str = "selene.compact";

static STATS_OUTPUTS: [StaticOutputColumn; 12] = [
    StaticOutputColumn::new("allocated_nodes", GqlType::Uint64)
        .with_description("Allocated node rows, including live and dead rows."),
    StaticOutputColumn::new("live_nodes", GqlType::Uint64).with_description("Alive node rows."),
    StaticOutputColumn::new("reclaimable_nodes", GqlType::Uint64)
        .with_description("Dead node rows reclaimable by compaction."),
    StaticOutputColumn::new("allocated_edges", GqlType::Uint64)
        .with_description("Allocated edge rows, including live and dead rows."),
    StaticOutputColumn::new("live_edges", GqlType::Uint64).with_description("Alive edge rows."),
    StaticOutputColumn::new("reclaimable_edges", GqlType::Uint64)
        .with_description("Dead edge rows reclaimable by compaction."),
    StaticOutputColumn::new("allocated_rows", GqlType::Uint64)
        .with_description("Allocated node plus edge rows."),
    StaticOutputColumn::new("live_rows", GqlType::Uint64)
        .with_description("Alive node plus edge rows."),
    StaticOutputColumn::new("reclaimable_rows", GqlType::Uint64)
        .with_description("Dead node plus edge rows reclaimable by compaction."),
    StaticOutputColumn::new("reclaimable_row_basis_points", GqlType::Uint64)
        .with_description("Reclaimable rows divided by allocated rows, scaled by 10,000."),
    StaticOutputColumn::new("compaction_recommended", GqlType::Boolean)
        .with_description("Whether row-space diagnostics recommend maintenance compaction."),
    StaticOutputColumn::new("dense", GqlType::Boolean)
        .with_description("Whether no dead rows remain to compact."),
];

static COMPACT_OUTPUTS: [StaticOutputColumn; 26] = [
    StaticOutputColumn::new("before_allocated_nodes", GqlType::Uint64)
        .with_description("Allocated node rows before compaction."),
    StaticOutputColumn::new("before_live_nodes", GqlType::Uint64)
        .with_description("Alive node rows before compaction."),
    StaticOutputColumn::new("before_reclaimable_nodes", GqlType::Uint64)
        .with_description("Dead node rows reclaimable before compaction."),
    StaticOutputColumn::new("before_allocated_edges", GqlType::Uint64)
        .with_description("Allocated edge rows before compaction."),
    StaticOutputColumn::new("before_live_edges", GqlType::Uint64)
        .with_description("Alive edge rows before compaction."),
    StaticOutputColumn::new("before_reclaimable_edges", GqlType::Uint64)
        .with_description("Dead edge rows reclaimable before compaction."),
    StaticOutputColumn::new("before_allocated_rows", GqlType::Uint64)
        .with_description("Allocated node plus edge rows before compaction."),
    StaticOutputColumn::new("before_live_rows", GqlType::Uint64)
        .with_description("Alive node plus edge rows before compaction."),
    StaticOutputColumn::new("before_reclaimable_rows", GqlType::Uint64)
        .with_description("Dead node plus edge rows reclaimable before compaction."),
    StaticOutputColumn::new("before_reclaimable_row_basis_points", GqlType::Uint64)
        .with_description("Before-compaction reclaimable row ratio scaled by 10,000."),
    StaticOutputColumn::new("before_compaction_recommended", GqlType::Boolean).with_description(
        "Whether row-space diagnostics recommended compaction before maintenance.",
    ),
    StaticOutputColumn::new("before_dense", GqlType::Boolean)
        .with_description("Whether the graph was dense before compaction."),
    StaticOutputColumn::new("reclaimed_nodes", GqlType::Uint64)
        .with_description("Node rows reclaimed by this compaction pass."),
    StaticOutputColumn::new("reclaimed_edges", GqlType::Uint64)
        .with_description("Edge rows reclaimed by this compaction pass."),
    StaticOutputColumn::new("after_allocated_nodes", GqlType::Uint64)
        .with_description("Allocated node rows after compaction."),
    StaticOutputColumn::new("after_live_nodes", GqlType::Uint64)
        .with_description("Alive node rows after compaction."),
    StaticOutputColumn::new("after_reclaimable_nodes", GqlType::Uint64)
        .with_description("Dead node rows reclaimable after compaction."),
    StaticOutputColumn::new("after_allocated_edges", GqlType::Uint64)
        .with_description("Allocated edge rows after compaction."),
    StaticOutputColumn::new("after_live_edges", GqlType::Uint64)
        .with_description("Alive edge rows after compaction."),
    StaticOutputColumn::new("after_reclaimable_edges", GqlType::Uint64)
        .with_description("Dead edge rows reclaimable after compaction."),
    StaticOutputColumn::new("after_allocated_rows", GqlType::Uint64)
        .with_description("Allocated node plus edge rows after compaction."),
    StaticOutputColumn::new("after_live_rows", GqlType::Uint64)
        .with_description("Alive node plus edge rows after compaction."),
    StaticOutputColumn::new("after_reclaimable_rows", GqlType::Uint64)
        .with_description("Dead node plus edge rows reclaimable after compaction."),
    StaticOutputColumn::new("after_reclaimable_row_basis_points", GqlType::Uint64)
        .with_description("After-compaction reclaimable row ratio scaled by 10,000."),
    StaticOutputColumn::new("after_compaction_recommended", GqlType::Boolean).with_description(
        "Whether row-space diagnostics still recommend compaction after maintenance.",
    ),
    StaticOutputColumn::new("after_dense", GqlType::Boolean)
        .with_description("Whether the graph is dense after compaction."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    let params: [StaticParameter; 0] = [];
    params
        .into_iter()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    STATS_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn compact_output_columns() -> Vec<ProcedureOutputColumn> {
    COMPACT_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute_stats(
    ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !args.is_empty() {
        return Err(ProcedureError::InvalidArgument {
            detail: format!("{STATS_PROC_NAME} expects zero arguments"),
        });
    }
    Ok(ProcedureResult {
        rows: vec![compaction_stats_values(ctx.snapshot().compaction_stats())],
    })
}

pub(super) fn execute_compact(
    ctx: &MaintenanceContext<'_, '_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !args.is_empty() {
        return Err(ProcedureError::InvalidArgument {
            detail: format!("{COMPACT_PROC_NAME} expects zero arguments"),
        });
    }
    ctx.cancellation_checker()
        .check()
        .map_err(cancellation_error)?;
    let before = ctx.compaction_stats();
    let report = ctx.compact().map_err(|source| ProcedureError::Internal {
        detail: format!("{COMPACT_PROC_NAME} failed: {source}"),
    })?;
    let after = ctx.compaction_stats();

    let mut row = compaction_stats_values(before);
    row.push(Value::Uint(report.reclaimed_nodes));
    row.push(Value::Uint(report.reclaimed_edges));
    row.extend(compaction_stats_values(after));
    Ok(ProcedureResult { rows: vec![row] })
}

fn cancellation_error(cause: CancellationCause) -> ProcedureError {
    match cause {
        CancellationCause::Cancelled => ProcedureError::Cancelled,
        CancellationCause::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
    }
}

fn compaction_stats_values(stats: CompactionStats) -> Vec<Value> {
    vec![
        Value::Uint(stats.allocated_nodes),
        Value::Uint(stats.live_nodes),
        Value::Uint(stats.reclaimable_nodes),
        Value::Uint(stats.allocated_edges),
        Value::Uint(stats.live_edges),
        Value::Uint(stats.reclaimable_edges),
        Value::Uint(stats.allocated_rows()),
        Value::Uint(stats.live_rows()),
        Value::Uint(stats.reclaimable_rows()),
        Value::Uint(stats.reclaimable_row_basis_points()),
        Value::Bool(stats.compaction_recommended()),
        Value::Bool(stats.is_dense()),
    ]
}

#[cfg(test)]
mod tests;
