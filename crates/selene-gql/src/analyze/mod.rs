//! Semantic analyzer entry points.
//!
//! BRIEF-21 landed name binding, BRIEF-22 added expression type inference, and
//! BRIEF-23 wires procedure signature metadata into CALL/YIELD analysis.
//! Mutation write-set checks and closed-graph schema validation layer onto
//! this module in later M5b briefs.

pub mod ast;
pub mod binding;
pub mod error;
pub mod scope;
pub mod types;

pub(crate) mod bind;
pub(crate) mod infer;

use crate::{ProcedureRegistry, Statement};

pub use ast::{AnalyzedStatement, AnalyzedStatementKind};
pub use binding::{BindingDecl, BindingDeclKind, BindingId, BindingUse, BindingUseKind};
pub use error::{
    AnalysisError, ConditionClause, ExpectedType, PatternElementKind, Side, TypeMismatchContext,
};
pub use scope::{BindingScope, BindingScopeTree, ScopeId, ScopeKind};
pub use types::{AnalyzedType, ExprId, ExprIdMap, ExprTypeTable};

/// Analyze a parsed GQL statement.
///
/// Resolves every binding reference, allocates [`BindingId`]s for every
/// declaration site, resolves procedure signatures through `registry`, and
/// returns an [`AnalyzedStatement`] suitable for the planner stage.
///
/// # Errors
///
/// Returns the first [`AnalysisError`] detected by the fail-fast bind pass.
pub fn analyze(
    stmt: Statement,
    registry: &dyn ProcedureRegistry,
) -> Result<AnalyzedStatement, AnalysisError> {
    bind::bind_statement(stmt, registry)
}
