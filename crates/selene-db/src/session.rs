//! Facade session context and selected-graph request leases.

use std::{cell::Cell, marker::PhantomData, sync::Arc};

use selene_catalog::GraphId as LowerGraphId;
use selene_gql::{CatalogSessionOutput, PreparedCatalogRequest};
use selene_graph::SharedGraph;

use crate::{
    CatalogReadSnapshot, Error, ExecutionOutcome, GeneralParameter, ObjectPath, Request,
    RequestContext, RequestOutcome, RequestParams, RequestTimestamp, Result, SessionContext,
    database::DatabaseInner,
    ddl,
    params::validated_parameter_name,
    transaction::{DatabaseDraft, require_committed},
};

/// Movable session selected to one catalog graph identity.
///
/// The session owns its database and a controlled [`SessionContext`], but no
/// runtime graph handle or catalog snapshot. Each call validates every copied
/// home/current stable identity, then rechecks the selected graph under a
/// temporary lifecycle read lease. A same-path recreation never rebinds the
/// session.
///
/// A session is `Send` but intentionally not `Sync`. It accepts one request at a
/// time and retains the actual active [`RequestContext`] in its request slot.
/// Sharing it across threads requires caller-owned synchronization; the facade
/// does not claim concurrent request execution.
///
/// ```compile_fail
/// fn require_sync<T: Sync>() {}
/// require_sync::<selene_db::Session>();
/// ```
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

    /// Return typed inspection of this session's dependencies and controlled state.
    #[must_use]
    pub const fn context(&self) -> &SessionContext {
        &self.context
    }

    /// Bind or replace one typed session parameter for subsequent requests.
    ///
    /// Request-scoped parameters shadow this dictionary by exact name without
    /// mutating it. A request already in progress keeps its immutable snapshot.
    ///
    /// # Errors
    ///
    /// Returns an invalid-name diagnostic when `name` would not parse after `$`.
    pub fn set_parameter(
        &self,
        name: &str,
        parameter: GeneralParameter,
    ) -> Result<Option<GeneralParameter>> {
        let name = validated_parameter_name(name)?;
        Ok(self.context.set_parameter(name, parameter))
    }

    /// Remove one exact-name session parameter.
    ///
    /// # Errors
    ///
    /// Returns an invalid-name diagnostic when `name` would not parse after `$`.
    pub fn remove_parameter(&self, name: &str) -> Result<Option<GeneralParameter>> {
        let name = validated_parameter_name(name)?;
        Ok(self.context.remove_parameter(name.as_str()))
    }

    /// Parse, plan, and execute one GQL statement.
    ///
    /// This compatibility entry point executes a [`Request`] with no
    /// request-scoped bindings and converts its [`RequestOutcome`] back to the
    /// existing `Result` shape. Session bindings still seed the request.
    ///
    /// # Errors
    ///
    /// Returns a facade-owned diagnostic for invalid GQL, unsupported stateful
    /// controls, a stale context reference, analysis/planning failures, or
    /// execution failures.
    pub fn execute(&self, source: &str) -> Result<ExecutionOutcome> {
        self.execute_request(Request::new(source)).into_result()
    }

    /// Execute one source statement with an explicit immutable request context.
    ///
    /// Session parameters are snapshotted first, then request parameters shadow
    /// exact names. The associated context remains active through graph
    /// execution and post-lease catalog dispatch. Every returned failure uses
    /// [`RequestOutcome::Failed`]. Panics are not caught; the request guard clears
    /// the slot while unwinding.
    #[must_use]
    pub fn execute_request(&self, request: Request) -> RequestOutcome {
        self.execute_request_with(request, RequestTimestamp::capture(), |_| {})
    }

    fn execute_request_with(
        &self,
        request: Request,
        timestamp: RequestTimestamp,
        active_hook: impl FnOnce(&Self),
    ) -> RequestOutcome {
        let merged =
            RequestParams::overlay(&self.context.parameter_snapshot(), request.parameters());
        let context = Arc::new(RequestContext::new(merged, timestamp));
        let guard = match self.context.activate_request(Arc::clone(&context)) {
            Ok(guard) => guard,
            Err(error) => return RequestOutcome::failed(context, error),
        };
        active_hook(self);
        let result = self.execute_active_request(&request, &context);
        drop(guard);
        match result {
            Ok(outcome) => RequestOutcome::Succeeded { context, outcome },
            Err(error) => RequestOutcome::failed(context, error),
        }
    }

    fn execute_active_request(
        &self,
        request: &Request,
        context: &RequestContext,
    ) -> Result<ExecutionOutcome> {
        self.validate_context_references()?;
        let current_graph = self.context.current_graph();
        let graph_id = LowerGraphId::new(current_graph.id.get()).map_err(|source| {
            Error::invalid_session_reference(Error::from_catalog_invariant(source))
        })?;
        let audit_bytes = self
            .context
            .principal()
            .and_then(crate::Principal::audit_bytes_arc);
        let input = context.lower_input(self.context.time_zone().seconds())?;
        match self.inner.prepare_catalog_request(
            graph_id,
            &current_graph.path,
            audit_bytes,
            request.source(),
            input,
        )? {
            PreparedSessionExecution::Complete(CatalogSessionOutput::RequestOutcome(output)) => {
                ExecutionOutcome::from_engine(output)
            }
            PreparedSessionExecution::Deferred(prepared) => {
                if let Some(command) = prepared.database_catalog_command().cloned() {
                    ddl::execute(&self.inner, &self.context.current_schema().path, command)
                } else {
                    self.inner.execute_prepared_mutation(
                        graph_id,
                        &current_graph.path,
                        self.context
                            .principal()
                            .and_then(crate::Principal::audit_bytes_arc),
                        prepared,
                    )
                }
            }
            PreparedSessionExecution::Complete(
                CatalogSessionOutput::Statement(_)
                | CatalogSessionOutput::DatabaseCatalog(_)
                | CatalogSessionOutput::Prepared(_),
            ) => Err(Error::unsupported_engine_outcome()),
            PreparedSessionExecution::Complete(_) => Err(Error::unsupported_engine_outcome()),
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
    /// Parse and plan once, then execute reads or defer mutations.
    ///
    /// A database-catalog command is returned unexecuted. The facade dispatches
    /// it only after this method releases the graph lease.
    fn prepare_catalog_request(
        &self,
        id: LowerGraphId,
        path: &ObjectPath,
        audit_bytes: Option<Arc<[u8]>>,
        source: &str,
        request: selene_gql::RequestExecutionInput,
    ) -> Result<PreparedSessionExecution> {
        self.with_graph_request(id, path, |graph| {
            let mut session = match audit_bytes {
                Some(audit_bytes) => selene_gql::Session::with_principal(graph, audit_bytes),
                None => selene_gql::Session::new(graph),
            };
            let prepared = session
                .prepare_source_catalog_request(source, &self.procedures, request)
                .map_err(Error::from_engine)?;
            if prepared.is_read_only() {
                session
                    .execute_prepared_catalog_request(prepared, &self.procedures)
                    .map(PreparedSessionExecution::Complete)
                    .map_err(Error::from_engine)
            } else {
                Ok(PreparedSessionExecution::Deferred(prepared))
            }
        })
    }

    fn execute_prepared_mutation(
        &self,
        id: LowerGraphId,
        _path: &ObjectPath,
        audit_bytes: Option<Arc<[u8]>>,
        prepared: PreparedCatalogRequest,
    ) -> Result<ExecutionOutcome> {
        self.with_mutation_reservation(|reservation| {
            let base = self.state.load_full();
            let mut draft = DatabaseDraft::new(&base, &reservation);
            let instance = draft.pin_graph(&base, id)?;
            let snapshot = instance.graph.read();
            let stale_plan = prepared.graph_id().get() != id.get()
                || prepared.graph_generation() != snapshot.meta.generation
                || prepared.schema_version() != instance.graph.schema_version();
            let scratch = SharedGraph::try_from_graph(snapshot.as_ref().clone())
                .map_err(Error::invalid_graph_type_source)?;
            drop(snapshot);
            let mut session = match audit_bytes {
                Some(audit_bytes) => selene_gql::Session::with_principal(&scratch, audit_bytes),
                None => selene_gql::Session::new(&scratch),
            };
            let prepared = if stale_plan {
                session
                    .reprepare_source_catalog_request(prepared, &self.procedures)
                    .map_err(Error::from_engine)?
            } else {
                prepared
            };
            if prepared.is_read_only() || prepared.is_database_catalog() {
                return Err(Error::catalog_invariant(
                    "selected mutation changed category while replanning",
                ));
            }
            let staged = session
                .execute_prepared_catalog_request_unpublished(prepared, &self.procedures)
                .map_err(Error::from_engine)?;
            let (output, prepared_graph) = staged.into_parts();
            drop(session);
            drop(scratch);

            draft.attach_prepared_graph(id, prepared_graph)?;
            require_committed(self.publish_database_draft(reservation, draft)?)?;
            ExecutionOutcome::from_engine(output)
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

enum PreparedSessionExecution {
    Complete(CatalogSessionOutput),
    Deferred(PreparedCatalogRequest),
}

#[cfg(test)]
mod tests {
    use std::{panic::AssertUnwindSafe, sync::Arc};

    use crate::{
        CreatePolicy, Database, ErrorKind, ObjectPath, Request, RequestOutcome, RequestSlotState,
        RequestTimestamp, SchemaPath,
    };

    fn session() -> super::Session {
        let database = Database::builder().build();
        let catalog = database.catalog();
        let schema = SchemaPath::regular("selene", "request_guard").unwrap();
        let graph = ObjectPath::regular("selene", "request_guard", "main").unwrap();
        catalog
            .create_schema(&schema, CreatePolicy::Strict)
            .unwrap();
        catalog
            .create_graph(&graph, None, CreatePolicy::Strict)
            .unwrap();
        database.session(&graph).unwrap()
    }

    #[test]
    fn active_slot_holds_actual_context_and_rejects_reentry() {
        let session = session();
        let timestamp = RequestTimestamp::from_parts(1_788_692_096, 123_456_789);
        let outcome =
            session.execute_request_with(Request::new("RETURN 1"), timestamp, |session| {
                assert_eq!(session.context().request_slot(), RequestSlotState::Active);
                let active = session
                    .context()
                    .current_request()
                    .expect("request is associated");
                assert_eq!(active.timestamp(), timestamp);

                let nested = session.execute_request(Request::new("RETURN 2"));
                assert_eq!(
                    nested.error().map(crate::Error::kind),
                    Some(ErrorKind::RequestAlreadyActive)
                );
                let still_active = session.context().current_request().unwrap();
                assert!(Arc::ptr_eq(&active, &still_active));
            });

        assert!(matches!(outcome, RequestOutcome::Succeeded { .. }));
        assert_eq!(session.context().request_slot(), RequestSlotState::Vacant);
        assert!(session.context().current_request().is_none());
    }

    #[test]
    fn panic_propagates_clears_slot_and_session_is_reusable() {
        let session = session();
        let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
            session.execute_request_with(
                Request::new("RETURN 1"),
                RequestTimestamp::from_parts(1_788_692_096, 0),
                |_| panic!("injected request panic"),
            )
        }));

        assert!(panic.is_err());
        assert_eq!(session.context().request_slot(), RequestSlotState::Vacant);
        assert!(matches!(
            session.execute_request(Request::new("RETURN 1")),
            RequestOutcome::Succeeded { .. }
        ));
    }
}
