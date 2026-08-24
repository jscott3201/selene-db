//! Lifetime-free selected-graph facade session and request leases.

use std::sync::Arc;

use selene_catalog::GraphId as LowerGraphId;
use selene_gql::CatalogSessionOutput;

use crate::{
    Error, ExecutionOutcome, ObjectPath, Result, SchemaPath, database::DatabaseInner, ddl,
};

/// Movable session selected to one catalog graph identity.
///
/// The session owns its database and stores no runtime graph handle. Each call
/// revalidates the selected stable identity under a temporary graph lifecycle
/// read lease, then creates a fresh lower executor session. A drop or
/// replacement makes the session stale; recreating the same path never rebinds
/// it to the replacement.
///
/// The session is stateless between requests. Persistent transaction and
/// request context belong to M03.
///
/// Transaction and `SESSION` controls return feature-not-supported instead of
/// reporting state that would disappear after the call. Database-catalog
/// statements are parsed through the selected graph and dispatched after its
/// request lease is released. Relative graph and graph-type references resolve
/// against the selected graph's schema. Successful catalog statements return
/// [`ExecutionOutcome::OmittedResult`]; their failures carry the same
/// [`ErrorKind`](crate::ErrorKind) and GQLSTATUS as the equivalent
/// [`Catalog`](crate::Catalog) call.
pub struct Session {
    inner: Arc<DatabaseInner>,
    graph_id: LowerGraphId,
    graph_path: ObjectPath,
    schema_path: SchemaPath,
}

impl Session {
    pub(crate) fn new(
        inner: Arc<DatabaseInner>,
        graph_id: LowerGraphId,
        graph_path: ObjectPath,
    ) -> Self {
        let schema_path = graph_path.schema_path();
        Self {
            inner,
            graph_id,
            graph_path,
            schema_path,
        }
    }

    /// Parse, plan, and execute one GQL statement.
    ///
    /// The result is summary-only. Row values, parameters, and facade
    /// transactions are added by later milestones.
    ///
    /// # Errors
    ///
    /// Returns a facade-owned diagnostic for invalid GQL, unsupported stateful
    /// controls, a stale graph selection, analysis/planning failures, or
    /// execution failures.
    pub fn execute(&self, source: &str) -> Result<ExecutionOutcome> {
        match self
            .inner
            .execute_catalog_session(self.graph_id, &self.graph_path, source)?
        {
            CatalogSessionOutput::Statement(output) => ExecutionOutcome::from_engine(output),
            CatalogSessionOutput::DatabaseCatalog(command) => {
                ddl::execute(&self.inner, &self.schema_path, command)
            }
            _ => Err(Error::unsupported_engine_outcome()),
        }
    }
}

impl DatabaseInner {
    /// Parse, plan, and either execute or intercept one selected-session
    /// statement under the graph's request lease.
    ///
    /// A database-catalog command is returned unexecuted. The facade dispatches
    /// it only after this method releases the graph lease.
    fn execute_catalog_session(
        &self,
        id: LowerGraphId,
        path: &ObjectPath,
        source: &str,
    ) -> Result<CatalogSessionOutput> {
        self.with_graph_request(id, path, |graph| {
            selene_gql::Session::new(graph)
                .execute_source_catalog_session(source, &self.procedures)
                .map_err(Error::from_engine)
        })
    }

    pub(crate) fn with_graph_request<T>(
        &self,
        id: LowerGraphId,
        path: &impl std::fmt::Display,
        execute: impl FnOnce(&selene_graph::SharedGraph) -> Result<T>,
    ) -> Result<T> {
        let observed = self.state.load_full();
        let instance = observed
            .graphs
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::stale_graph(path))?;
        #[cfg(test)]
        let _depth = crate::database::GraphRequestDepth::enter();
        let _lease = instance.lifecycle.read();
        let current = self.state.load_full();
        if current
            .graphs
            .get(&id)
            .is_none_or(|registered| !Arc::ptr_eq(registered, &instance))
        {
            return Err(Error::stale_graph(path));
        }
        execute(&instance.graph)
    }
}
