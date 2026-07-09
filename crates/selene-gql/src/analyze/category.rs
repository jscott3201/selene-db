//! Statement classification for transaction-mode enforcement.

use crate::{DdlStatement, ProcedureMutability, ProcedureRegistry, Statement};

/// Statement-level classification consumed by the runtime transaction-state
/// machine to enforce catalog/data mixing rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatementCategory {
    /// Query-only statement.
    ReadOnly,
    /// Statement that may modify graph data.
    DataModifying,
    /// Statement that may modify graph/catalog metadata.
    CatalogModifying,
    /// Statement that may rebuild derived engine state.
    Maintenance,
    /// Transaction-control statement.
    TransactionControl,
    /// Session-control statement (ISO/IEC 39075:2024 section 7).
    SessionControl,
}

pub(crate) fn classify(
    statement: &Statement,
    registry: &dyn ProcedureRegistry,
) -> StatementCategory {
    match statement {
        Statement::Query(_) | Statement::Composite { .. } | Statement::Chained { .. } => {
            StatementCategory::ReadOnly
        }
        Statement::Mutate(_) => StatementCategory::DataModifying,
        Statement::Ddl(statement) => classify_ddl(statement),
        Statement::Call(call) => registry
            .lookup(&call.name)
            .map(|metadata| classify_mutability(metadata.mutability))
            .unwrap_or(StatementCategory::ReadOnly),
        Statement::Explain { .. } => StatementCategory::ReadOnly,
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. } => StatementCategory::TransactionControl,
        Statement::SessionSetValue { .. }
        | Statement::SessionSetTimeZone { .. }
        | Statement::SessionSetGraph { .. }
        | Statement::SessionReset { .. }
        | Statement::SessionClose { .. } => StatementCategory::SessionControl,
    }
}

const fn classify_ddl(statement: &DdlStatement) -> StatementCategory {
    match statement {
        DdlStatement::ShowNodeTypes(_)
        | DdlStatement::ShowEdgeTypes(_)
        | DdlStatement::ShowIndexes(_)
        | DdlStatement::ShowProcedures(_) => StatementCategory::ReadOnly,
        DdlStatement::CreateGraph { .. }
        | DdlStatement::DropGraph { .. }
        | DdlStatement::CreateNodeType { .. }
        | DdlStatement::CreateEdgeType { .. }
        | DdlStatement::AlterNodeType { .. }
        | DdlStatement::AlterEdgeType { .. }
        | DdlStatement::DropNodeType { .. }
        | DdlStatement::DropEdgeType { .. }
        | DdlStatement::CreateIndex { .. }
        | DdlStatement::DropIndex { .. } => StatementCategory::CatalogModifying,
        // TRUNCATE removes data instances while keeping the catalog type intact,
        // so it is DataModifying (BRIEF-150). Both categories route through the
        // same write-transaction execution arm, so this only sharpens the
        // semantic label.
        DdlStatement::TruncateNodeType { .. } | DdlStatement::TruncateEdgeType { .. } => {
            StatementCategory::DataModifying
        }
    }
}

const fn classify_mutability(mutability: ProcedureMutability) -> StatementCategory {
    match mutability {
        ProcedureMutability::Read => StatementCategory::ReadOnly,
        ProcedureMutability::SchemaWrite => StatementCategory::CatalogModifying,
        ProcedureMutability::MaintenanceWrite => StatementCategory::Maintenance,
    }
}
