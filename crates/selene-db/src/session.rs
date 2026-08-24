//! Facade session context and selected-graph request leases.

use std::{cell::Cell, marker::PhantomData, sync::Arc};

use selene_catalog::GraphId as LowerGraphId;
use selene_gql::CatalogSessionOutput;

use crate::{
    CatalogReadSnapshot, Error, ExecutionOutcome, ObjectPath, Result, SessionContext,
    database::DatabaseInner, ddl,
};

/// Movable session selected to one catalog graph identity.
///
/// The session owns its database and an immutable [`SessionContext`], but no
/// runtime graph handle or catalog snapshot. Each call validates every copied
/// home/current stable identity, then rechecks the selected graph under a
/// temporary lifecycle read lease. A same-path recreation never rebinds the
/// session.
///
/// A session is `Send` but intentionally not `Sync`. The current `execute(&self)`
/// signature is a compatibility boundary; request and transaction slots remain
/// vacant and carry no hidden interior-mutable state.
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
    context: SessionContext,
    not_sync: PhantomData<Cell<()>>,
}

impl Session {
    pub(crate) fn new(inner: Arc<DatabaseInner>, context: SessionContext) -> Self {
        Self {
            inner,
            context,
            not_sync: PhantomData,
        }
    }

    /// Return immutable typed inspection of this session's creation context.
    #[must_use]
    pub const fn context(&self) -> &SessionContext {
        &self.context
    }

    /// Parse, plan, and execute one GQL statement.
    ///
    /// The result is summary-only. Row values, parameters, and facade
    /// transactions are added by later milestones.
    ///
    /// # Errors
    ///
    /// Returns a facade-owned diagnostic for invalid GQL, unsupported stateful
    /// controls, a stale context reference, analysis/planning failures, or
    /// execution failures.
    pub fn execute(&self, source: &str) -> Result<ExecutionOutcome> {
        self.validate_context_references()?;
        let current_graph = self.context.current_graph();
        let graph_id = LowerGraphId::new(current_graph.id.get()).map_err(|source| {
            Error::invalid_session_reference(Error::from_catalog_invariant(source))
        })?;
        let audit_bytes = self
            .context
            .principal()
            .and_then(crate::Principal::audit_bytes_arc);
        match self.inner.execute_catalog_session(
            graph_id,
            &current_graph.path,
            audit_bytes,
            source,
        )? {
            CatalogSessionOutput::Statement(output) => ExecutionOutcome::from_engine(output),
            CatalogSessionOutput::DatabaseCatalog(command) => {
                ddl::execute(&self.inner, &self.context.current_schema().path, command)
            }
            _ => Err(Error::unsupported_engine_outcome()),
        }
    }

    fn validate_context_references(&self) -> Result<()> {
        let snapshot = CatalogReadSnapshot {
            state: self.inner.state.load_full(),
        };
        let references_are_current = snapshot
            .matches_schema_reference(self.context.current_schema())
            && snapshot.matches_graph_reference(self.context.current_graph())
            && self
                .context
                .home_schema()
                .is_none_or(|schema| snapshot.matches_schema_reference(schema))
            && self
                .context
                .home_graph()
                .is_none_or(|graph| snapshot.matches_graph_reference(graph));
        if references_are_current {
            Ok(())
        } else {
            Err(Error::stale_session_reference())
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
        audit_bytes: Option<Arc<[u8]>>,
        source: &str,
    ) -> Result<CatalogSessionOutput> {
        self.with_graph_request(id, path, |graph| {
            let mut session = match audit_bytes {
                Some(audit_bytes) => selene_gql::Session::with_principal(graph, audit_bytes),
                None => selene_gql::Session::new(graph),
            };
            session
                .execute_source_catalog_session(source, &self.procedures)
                .map_err(Error::from_engine)
        })
    }

    pub(crate) fn with_graph_request<T>(
        &self,
        id: LowerGraphId,
        _path: &impl std::fmt::Display,
        execute: impl FnOnce(&selene_graph::SharedGraph) -> Result<T>,
    ) -> Result<T> {
        let observed = self.state.load_full();
        let instance = observed
            .graphs
            .get(&id)
            .cloned()
            .ok_or_else(Error::stale_session_reference)?;
        #[cfg(test)]
        let _depth = crate::database::GraphRequestDepth::enter();
        let _lease = instance.lifecycle.read();
        let current = self.state.load_full();
        if current
            .graphs
            .get(&id)
            .is_none_or(|registered| !Arc::ptr_eq(registered, &instance))
        {
            return Err(Error::stale_session_reference());
        }
        execute(&instance.graph)
    }
}
