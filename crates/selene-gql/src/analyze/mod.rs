//! Semantic analyzer entry points.
//!
//! BRIEF-21 lands the name-binding pass only. Type inference, procedure
//! signature validation, mutation write-set checks, and closed-graph schema
//! validation layer onto this module in the following M5b briefs.

pub mod ast;
pub mod binding;
pub mod error;
pub mod scope;
pub mod types;

pub(crate) mod bind;

use crate::Statement;

pub use ast::{AnalyzedStatement, AnalyzedStatementKind};
pub use binding::{BindingDecl, BindingDeclKind, BindingId, BindingUse, BindingUseKind};
pub use error::{AnalysisError, PatternElementKind};
pub use scope::{BindingScope, BindingScopeTree, ScopeId, ScopeKind};
pub use types::AnalyzedType;

/// Analyze a parsed GQL statement.
///
/// Resolves every binding reference, allocates [`BindingId`]s for every
/// declaration site, and returns an [`AnalyzedStatement`] suitable for the
/// planner stage. BRIEF-21 intentionally performs name binding only; every
/// expression type cell is [`AnalyzedType::Dynamic`].
///
/// # Errors
///
/// Returns the first [`AnalysisError`] detected by the fail-fast bind pass.
pub fn analyze(stmt: Statement) -> Result<AnalyzedStatement, AnalysisError> {
    bind::bind_statement(stmt)
}
