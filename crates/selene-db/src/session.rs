//! Lifetime-free facade session.

use std::sync::Arc;

use selene_core::Change;

use crate::{ExecutionOutcome, Result, database::DatabaseInner};

/// Movable session that owns its database through shared ownership.
///
/// This bootstrap session is deliberately stateless between requests. Each
/// call borrows the current graph through a temporary lower executor session.
/// Transaction and `SESSION` controls return feature-not-supported instead of
/// reporting state that would disappear after the call. M03 owns persistent
/// session and transaction state.
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
        let mut session = selene_gql::Session::new(&self.inner.bootstrap.graph);
        let output = session
            .execute_source_stateless(source, &self.inner.procedures)
            .map_err(crate::Error::from_engine)?;

        if let selene_gql::StatementOutput::Written(write) = &output
            && write
                .changes
                .iter()
                .any(|change| matches!(change, Change::GraphReset {}))
        {
            self.inner
                .procedures
                .forget_graph(self.inner.bootstrap.catalog.graph_id());
        }

        ExecutionOutcome::from_engine(output)
    }
}
