//! Single-parse selected-facade request preparation and staged execution.

use std::{mem, panic::AssertUnwindSafe, sync::Arc};

use selene_graph::write_txn::PreparedGraphCommit;

use crate::{DatabaseCatalogCommand, ExecutionPlan, ProcedureRegistry, StatementCategory};

use super::{
    CatalogSessionOutput, ExecutionOutcome as RuntimeExecutionOutcome, ExecutorError,
    RequestExecutionInput, Session, SessionParameterValue,
    request_runtime::RequestRuntime,
    statement::{SourceExecutionPolicy, database_catalog_command, execute_source_plan},
};

/// One owned selected-facade plan plus its exact request input.
///
/// Parsing, analysis, request validation, planning, and optimization have
/// already completed. The facade can execute this plan under the live graph
/// lease for reads, or under its global mutation reservation on a scratch graph
/// for writes, without a second parse.
#[doc(hidden)]
pub struct PreparedCatalogRequest {
    source: Arc<str>,
    plan: Arc<ExecutionPlan>,
    request: RequestExecutionInput,
    graph_id: selene_core::GraphId,
    graph_generation: u64,
    schema_version: u64,
}

impl PreparedCatalogRequest {
    /// Return whether this is exactly one database-catalog command.
    #[must_use]
    pub fn is_database_catalog(&self) -> bool {
        database_catalog_command(&self.plan).is_some()
    }

    /// Borrow the intercepted database-catalog command, when present.
    #[must_use]
    pub fn database_catalog_command(&self) -> Option<&DatabaseCatalogCommand> {
        database_catalog_command(&self.plan)
    }

    /// Return whether the prepared statement is read-only.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.plan.category == StatementCategory::ReadOnly
    }

    /// Return the graph identity against which this request was planned.
    #[must_use]
    pub const fn graph_id(&self) -> selene_core::GraphId {
        self.graph_id
    }

    /// Return the graph generation against which this request was planned.
    #[must_use]
    pub const fn graph_generation(&self) -> u64 {
        self.graph_generation
    }

    /// Return the graph schema epoch against which this request was planned.
    #[must_use]
    pub const fn schema_version(&self) -> u64 {
        self.schema_version
    }
}

/// Existing request outcome paired with an unpublished graph commit.
#[doc(hidden)]
pub struct PreparedCatalogMutationOutput {
    output: RuntimeExecutionOutcome,
    graph: PreparedGraphCommit,
}

impl PreparedCatalogMutationOutput {
    /// Consume the staging result into the existing outcome and graph bundle.
    #[must_use]
    pub fn into_parts(self) -> (RuntimeExecutionOutcome, PreparedGraphCommit) {
        (self.output, self.graph)
    }
}

impl<'g> Session<'g> {
    /// Compile one selected-facade request without executing its plan.
    #[doc(hidden)]
    pub fn prepare_source_catalog_request(
        &mut self,
        source: &str,
        registry: &dyn ProcedureRegistry,
        request: RequestExecutionInput,
    ) -> Result<PreparedCatalogRequest, ExecutorError> {
        let snapshot = self.graph().read();
        let graph_id = snapshot.graph_id();
        let graph_generation = snapshot.meta.generation;
        drop(snapshot);
        let schema_version = self.graph().schema_version();
        let (result, request, _, prepared_graph) =
            self.with_facade_request(request, false, |session| {
                session.execute_source_with_policy(
                    source,
                    registry,
                    SourceExecutionPolicy::PrepareCatalogSession,
                )
            });
        debug_assert!(prepared_graph.is_none());
        let CatalogSessionOutput::Prepared(plan) = result? else {
            return Err(ExecutorError::ImplementationDefined {
                detail: "selected request preparation did not return an owned plan",
            });
        };
        Ok(PreparedCatalogRequest {
            source: Arc::from(source),
            plan,
            request,
            graph_id,
            graph_generation,
            schema_version,
        })
    }

    /// Recompile a stale prepared request against this session's graph.
    #[doc(hidden)]
    pub fn reprepare_source_catalog_request(
        &mut self,
        prepared: PreparedCatalogRequest,
        registry: &dyn ProcedureRegistry,
    ) -> Result<PreparedCatalogRequest, ExecutorError> {
        self.prepare_source_catalog_request(&prepared.source, registry, prepared.request)
    }

    /// Execute a prepared request without enabling unpublished writes.
    #[doc(hidden)]
    pub fn execute_prepared_catalog_request(
        &mut self,
        prepared: PreparedCatalogRequest,
        registry: &dyn ProcedureRegistry,
    ) -> Result<CatalogSessionOutput, ExecutorError> {
        let (result, _, runtime, prepared_graph) =
            self.with_facade_request(prepared.request, false, |session| {
                execute_source_plan(
                    &prepared.plan,
                    session,
                    registry,
                    SourceExecutionPolicy::CatalogSession,
                )
            });
        debug_assert!(prepared_graph.is_none());
        result.map(|output| request_output(output, runtime))
    }

    /// Execute a prepared mutation and return its unpublished graph snapshot.
    #[doc(hidden)]
    pub fn execute_prepared_catalog_request_unpublished(
        &mut self,
        prepared: PreparedCatalogRequest,
        registry: &dyn ProcedureRegistry,
    ) -> Result<PreparedCatalogMutationOutput, ExecutorError> {
        if prepared.is_database_catalog()
            || !matches!(
                prepared.plan.category,
                StatementCategory::DataModifying | StatementCategory::CatalogModifying
            )
        {
            return Err(ExecutorError::ImplementationDefined {
                detail: "unpublished execution requires an engine graph mutation plan",
            });
        }
        let (result, _, runtime, prepared_graph) =
            self.with_facade_request(prepared.request, true, |session| {
                execute_source_plan(
                    &prepared.plan,
                    session,
                    registry,
                    SourceExecutionPolicy::CatalogSession,
                )
            });
        let output = request_output(result?, runtime);
        let CatalogSessionOutput::RequestOutcome(output) = output else {
            return Err(ExecutorError::ImplementationDefined {
                detail: "unpublished mutation did not return a request outcome",
            });
        };
        let graph = prepared_graph.ok_or(ExecutorError::ImplementationDefined {
            detail: "unpublished mutation did not prepare a graph commit",
        })?;
        Ok(PreparedCatalogMutationOutput { output, graph })
    }

    pub(super) fn with_facade_request<T>(
        &mut self,
        request: RequestExecutionInput,
        prepare_unpublished: bool,
        execute: impl FnOnce(&mut Self) -> Result<T, ExecutorError>,
    ) -> (
        Result<T, ExecutorError>,
        RequestExecutionInput,
        Arc<RequestRuntime>,
        Option<PreparedGraphCommit>,
    ) {
        let request_runtime = request.runtime();
        let scalar_parameters: std::collections::BTreeMap<_, _> = request
            .parameters
            .iter()
            .map(|(name, parameter)| (name.clone(), parameter.value().clone()))
            .collect();
        let parameters = scalar_parameters
            .iter()
            .map(|(name, value)| (name.clone(), SessionParameterValue::Scalar(value.clone())))
            .collect();
        let prior_parameters = mem::replace(&mut self.parameters, parameters);
        let prior_scalar_parameters = mem::replace(&mut self.scalar_parameters, scalar_parameters);
        let prior_time_zone = self.time_zone.replace(request.time_zone.clone());
        let prior_request = self.request.replace(request);
        let prior_prepare = mem::replace(&mut self.prepare_unpublished, prepare_unpublished);
        let prior_prepared = self.prepared_graph.take();

        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| execute(self)));

        self.parameters = prior_parameters;
        self.scalar_parameters = prior_scalar_parameters;
        self.time_zone = prior_time_zone;
        let request = self
            .request
            .take()
            .expect("facade request remains installed during execution");
        self.request = prior_request;
        self.prepare_unpublished = prior_prepare;
        let prepared = self.prepared_graph.take();
        self.prepared_graph = prior_prepared;

        match outcome {
            Ok(result) => (result, request, request_runtime, prepared),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

fn request_output(
    output: CatalogSessionOutput,
    runtime: Arc<RequestRuntime>,
) -> CatalogSessionOutput {
    match output {
        CatalogSessionOutput::Statement(output) => CatalogSessionOutput::RequestOutcome(
            RuntimeExecutionOutcome::from_statement(output, runtime.statuses()),
        ),
        output => output,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use selene_core::GraphId;
    use selene_graph::SharedGraph;

    use super::*;
    use crate::{EmptyProcedureRegistry, GqlStatus};

    fn request() -> RequestExecutionInput {
        RequestExecutionInput::new(
            BTreeMap::new(),
            jiff::Timestamp::new(1_788_692_096, 0).unwrap(),
            jiff::tz::TimeZone::UTC,
        )
    }

    #[test]
    fn prepared_mutation_executes_without_publishing_scratch_graph() {
        let graph = SharedGraph::new(GraphId::new(61));
        let registry = EmptyProcedureRegistry;
        let mut session = Session::with_principal(&graph, Arc::from([4_u8, 5]));
        let prepared = session
            .prepare_source_catalog_request("INSERT (:Prepared) RETURN 1", &registry, request())
            .unwrap();
        assert_eq!(prepared.graph_id(), GraphId::new(61));
        assert_eq!(prepared.graph_generation(), 0);

        let staged = session
            .execute_prepared_catalog_request_unpublished(prepared, &registry)
            .unwrap();
        let (output, prepared_graph) = staged.into_parts();

        assert_eq!(graph.read().node_count(), 0);
        assert_eq!(graph.read().meta.generation, 0);
        assert_eq!(prepared_graph.snapshot().node_count(), 1);
        assert_eq!(prepared_graph.snapshot().meta.generation, 1);
        assert_eq!(
            prepared_graph.outcome().principal.as_deref(),
            Some(&[4, 5][..])
        );
        assert!(matches!(output, RuntimeExecutionOutcome::Written { .. }));
    }

    #[test]
    fn selected_maintenance_is_rejected_during_preparation() {
        let graph = SharedGraph::new(GraphId::new(62));
        let registry = crate::BuiltinProcedureRegistry::new();
        let mut session = Session::new(&graph);

        let error = session
            .prepare_source_catalog_request("CALL selene.compact()", &registry, request())
            .err()
            .expect("selected maintenance is rejected before execution");

        assert_eq!(error.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
        assert_eq!(graph.read().meta.generation, 0);
    }
}
