//! Semantic analyzer entry points.
//!
//! BRIEF-21 landed name binding, BRIEF-22 added expression type inference, and
//! BRIEF-23 wires procedure signature metadata into CALL/YIELD analysis.
//! Mutation write-set checks and closed-graph schema validation layer on top of
//! the binding and type passes.

pub mod ast;
pub mod binding;
pub mod category;
pub mod error;
pub mod scope;
pub mod types;
pub mod write_set;

pub(crate) mod bind;
pub(crate) mod infer;
pub(crate) mod schema;

use selene_graph::GraphTypeDef;

use crate::{ProcedureRegistry, Statement};

pub use ast::{AnalyzedStatement, AnalyzedStatementKind};
pub use binding::{BindingDecl, BindingDeclKind, BindingId, BindingUse, BindingUseKind};
pub use category::StatementCategory;
pub use error::{
    AnalysisError, ConditionClause, ExpectedType, InvalidLabelForm, PatternElementKind, Side,
    TypeMismatchContext,
};
pub use scope::{BindingScope, BindingScopeTree, ScopeId, ScopeKind};
pub use types::{AnalyzedType, ExprId, ExprIdLookup, ExprTypeTable};
pub use write_set::{ElementKind, MutationWriteSet, WriteKind, WriteSetEntry};

/// Analyze a parsed GQL statement.
///
/// Resolves every binding reference, allocates [`BindingId`]s for every
/// declaration site, resolves procedure signatures through `registry`, applies
/// closed-graph static validation when `schema` is present, and returns an
/// [`AnalyzedStatement`] suitable for the planner stage.
///
/// # Errors
///
/// Returns the first [`AnalysisError`] detected by the fail-fast bind pass.
pub fn analyze(
    stmt: Statement,
    registry: &dyn ProcedureRegistry,
    schema: Option<&GraphTypeDef>,
) -> Result<AnalyzedStatement, AnalysisError> {
    let analyzed = bind::bind_statement(stmt, registry)?;
    if let Some(graph_type) = schema {
        self::schema::validate(&analyzed, graph_type)?;
    }
    Ok(analyzed)
}
