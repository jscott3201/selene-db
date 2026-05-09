//! Procedure-call planner IR rows.

use selene_core::IStr;

use crate::{ProcedureHandle, ProcedureMutability, ProcedureOutputSchema, SourceSpan};

use super::ProjectExpr;

/// Planned procedure call.
#[derive(Clone, Debug)]
pub struct PlannedCall {
    /// Qualified procedure name.
    pub procedure: Box<[IStr]>,
    /// Opaque procedure handle.
    pub handle: ProcedureHandle,
    /// Planned call arguments.
    pub args: Vec<ProjectExpr>,
    /// Requested yield columns.
    pub yield_cols: Vec<PlannedYieldItem>,
    /// Procedure output schema.
    pub output_schema: ProcedureOutputSchema,
    /// Procedure mutability class.
    pub mutability: ProcedureMutability,
    /// Source span.
    pub span: SourceSpan,
}

/// Planned yield item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedYieldItem {
    /// Source output column selector.
    pub column: YieldKind,
    /// Output alias, when present.
    pub alias: Option<IStr>,
    /// Source span of this yield item.
    pub span: SourceSpan,
}

/// Planned yield column selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YieldKind {
    /// `YIELD *`.
    Star,
    /// `YIELD col`.
    Named(IStr),
}
