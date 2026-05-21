//! Semantic analyzer entry points.
//!
//! The analyzer turns parsed statements into a closed semantic model: every
//! reference resolves to a `BindingDecl`, every `ValueExpr` has an expression
//! type cell, closed-graph mutations are statically validated when a schema is
//! supplied, and CALL arguments/YIELD bindings are checked against registry
//! metadata. It defers physical access selection and row-shape execution to the
//! planner/runtime boundary, and it treats dynamic expression cells as explicit
//! unknowns rather than implicit success. See Spec 08 §5.

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
#[tracing::instrument(
    name = "selene.gql.analyze",
    skip(stmt, registry, schema),
    fields(schema_bound = schema.is_some())
)]
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
