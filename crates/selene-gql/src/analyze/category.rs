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
    /// Transaction-control statement.
    TransactionControl,
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
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. } => StatementCategory::TransactionControl,
    }
}

const fn classify_ddl(statement: &DdlStatement) -> StatementCategory {
    match statement {
        DdlStatement::ShowNodeTypes(_) | DdlStatement::ShowEdgeTypes(_) => {
            StatementCategory::ReadOnly
        }
        DdlStatement::CreateGraph { .. }
        | DdlStatement::DropGraph { .. }
        | DdlStatement::CreateNodeType { .. }
        | DdlStatement::CreateEdgeType { .. }
        | DdlStatement::DropNodeType { .. }
        | DdlStatement::DropEdgeType { .. } => StatementCategory::CatalogModifying,
    }
}

const fn classify_mutability(mutability: ProcedureMutability) -> StatementCategory {
    match mutability {
        ProcedureMutability::Read => StatementCategory::ReadOnly,
        ProcedureMutability::GraphWrite => StatementCategory::DataModifying,
        ProcedureMutability::SchemaWrite | ProcedureMutability::Admin => {
            StatementCategory::CatalogModifying
        }
    }
}
