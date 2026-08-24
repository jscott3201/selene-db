//! Lifetime-free facade session.

use std::sync::Arc;

use selene_gql::FacadeOutput;

use crate::{
    Error, ExecutionOutcome, Result,
    database::{DatabaseInner, bootstrap_graph_id},
    ddl,
};

/// Movable session that owns its database through shared ownership.
///
/// This bootstrap session is deliberately stateless between requests. Each
/// call borrows the current graph through a temporary lower executor session.
/// Transaction and `SESSION` controls return feature-not-supported instead of
/// reporting state that would disappear after the call. M03 owns persistent
/// session and transaction state.
///
/// Database-catalog statements (`CREATE/DROP SCHEMA`, `CREATE/DROP GRAPH`)
/// are parsed through the same lower session, then dispatched to the catalog
/// service after the graph request lease is released. Relative graph
/// references resolve against the fixed current working schema
/// `/selene/public`. Successful catalog statements return
/// [`ExecutionOutcome::OmittedResult`]; their failures carry the same
/// [`ErrorKind`](crate::ErrorKind) and GQLSTATUS as the equivalent
/// [`Catalog`](crate::Catalog) call.
pub struct Session {
    inner: Arc<DatabaseInner>,
}

impl Session {
    pub(crate) const fn new(inner: Arc<DatabaseInner>) -> Self {
        Self { inner }
    }

    /// Parse, plan, and execute one GQL statement.
    ///
    /// The result is summary-only. Row values, parameters, and facade
    /// transactions are added by later milestones.
    ///
    /// # Errors
    ///
    /// Returns a facade-owned diagnostic for invalid GQL, unsupported stateful
    /// controls, analysis/planning failures, or execution failures.
    pub fn execute(&self, source: &str) -> Result<ExecutionOutcome> {
        match self.inner.execute_bootstrap(source)? {
            FacadeOutput::Statement(output) => {
                self.inner.finish_lower_output(bootstrap_graph_id(), output)
            }
            FacadeOutput::DatabaseCatalog(command) => ddl::execute(&self.inner, command, source),
            _ => Err(Error::unsupported_engine_outcome()),
        }
    }
}
