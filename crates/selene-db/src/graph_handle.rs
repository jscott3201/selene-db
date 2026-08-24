//! Non-owning named graph handles and request leases.

use std::sync::Arc;

use selene_catalog::GraphId as LowerGraphId;
use selene_core::Change;
use selene_gql::FacadeOutput;

use crate::{
    Error, ExecutionOutcome, GraphId, ObjectPath, Result,
    catalog::core_graph_id,
    database::{DatabaseInner, bootstrap_graph_id},
};

/// Stable-identity handle to a named graph.
///
/// The handle stores no graph instance. Every request re-resolves its identity,
/// so a drop invalidates old handles and same-path recreation cannot alias one.
#[derive(Clone)]
pub struct GraphHandle {
    inner: Arc<DatabaseInner>,
    id: LowerGraphId,
    path: ObjectPath,
}

impl GraphHandle {
    pub(crate) const fn new(inner: Arc<DatabaseInner>, id: LowerGraphId, path: ObjectPath) -> Self {
        Self { inner, id, path }
    }

    /// Return the stable catalog graph identity.
    #[must_use]
    pub const fn id(&self) -> GraphId {
        GraphId(self.id.get())
    }

    /// Return the absolute path recorded when the handle was opened.
    #[must_use]
    pub const fn path(&self) -> &ObjectPath {
        &self.path
    }

    /// Execute an ordinary stateless read or data mutation.
    ///
    /// Catalog, transaction-control, and session-control statements are
    /// rejected before execution. Catalog DDL must use the catalog service.
    pub fn execute(&self, source: &str) -> Result<ExecutionOutcome> {
        self.inner.execute_graph(self.id, &self.path, source, true)
    }
}

impl DatabaseInner {
    /// Parse, plan, and either execute or intercept one compatibility-session
    /// statement under the bootstrap graph's request lease.
    ///
    /// A database-catalog command is returned unexecuted; the caller dispatches
    /// it after this lease is released (see [`crate::ddl`]).
    pub(crate) fn execute_bootstrap(&self, source: &str) -> Result<FacadeOutput> {
        let path = self.bootstrap_path();
        self.with_graph_request(bootstrap_graph_id(), &path, |graph| {
            selene_gql::Session::new(graph)
                .execute_source_facade(source, &self.procedures)
                .map_err(Error::from_engine)
        })
    }

    fn bootstrap_path(&self) -> String {
        format!(
            "/{}/{}/{}",
            self.bootstrap.catalog_name(),
            self.bootstrap.schema_name(),
            self.bootstrap.graph_name()
        )
    }

    /// Execute one statement through a temporary lower session.
    ///
    /// `named` selects the named-graph policy, which rejects catalog and
    /// stateful control statements. The stateless policy is used by the
    /// bootstrap `DROP GRAPH` factory-reset bridge only.
    pub(crate) fn execute_graph(
        &self,
        id: LowerGraphId,
        path: &impl std::fmt::Display,
        source: &str,
        named: bool,
    ) -> Result<ExecutionOutcome> {
        let output = self.with_graph_request(id, path, |graph| {
            let mut session = selene_gql::Session::new(graph);
            if named {
                session.execute_source_named_graph(source, &self.procedures)
            } else {
                session.execute_source_stateless(source, &self.procedures)
            }
            .map_err(Error::from_engine)
        })?;
        self.finish_lower_output(id, output)
    }

    /// Map an executed lower output to the facade summary, clearing
    /// procedure-derived state after a graph reset.
    pub(crate) fn finish_lower_output(
        &self,
        id: LowerGraphId,
        output: selene_gql::StatementOutput,
    ) -> Result<ExecutionOutcome> {
        if let selene_gql::StatementOutput::Written(write) = &output
            && write
                .changes
                .iter()
                .any(|change| matches!(change, Change::GraphReset {}))
        {
            self.procedures.forget_graph(core_graph_id(id));
        }
        ExecutionOutcome::from_engine(output)
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
