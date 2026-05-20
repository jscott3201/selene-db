//! `selene.health` execution tests through the public GQL runtime.

use std::collections::HashMap;

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{
    ExecutionPlan, GqlType, ProcedureContext, ProcedureError, ProcedureHandle, ProcedureMetadata,
    ProcedureMutability, ProcedureOutputColumn, ProcedureOutputSchema, ProcedureParameter,
    ProcedureRegistry, ProcedureResult, ProcedureSignature, ProcedureTier, Session,
    StatementOutput, analyze, execute_statement, parse, plan,
};
use selene_graph::SharedGraph;
use selene_pack::ProcedurePackRegistry;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn health_source() -> &'static str {
    "CALL selene.health() YIELD graph_id, node_count, edge_count, schema_bound"
}

fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, registry, None).expect("test input analyzes");
    plan(&analyzed, registry).expect("test input plans")
}

fn execute(source: &str, graph: &SharedGraph, registry: &dyn ProcedureRegistry) -> StatementOutput {
    let plan = planned(source, registry);
    let mut session = Session::new(graph);
    execute_statement(&plan, &mut session, registry).expect("statement executes")
}

fn execute_with_session(
    source: &str,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> StatementOutput {
    let plan = planned(source, registry);
    execute_statement(&plan, session, registry).expect("statement executes")
}

fn row_values(output: StatementOutput) -> Vec<Value> {
    let StatementOutput::Rows(table) = output else {
        panic!("expected rows");
    };
    assert_eq!(table.row_count(), 1);
    table.rows()[0].values().to_vec()
}

#[test]
fn call_selene_health_yields_engine_state_columns() {
    let registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let graph = SharedGraph::new(GraphId::new(4101));

    let values = row_values(execute(health_source(), &graph, &registry));

    assert_eq!(
        values,
        vec![
            Value::Uint(4101),
            Value::Uint(0),
            Value::Uint(0),
            Value::Bool(false),
        ]
    );
}

#[test]
fn call_selene_health_yields_updated_counts_after_mutation() {
    let registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let graph = SharedGraph::new(GraphId::new(4102));
    let mut txn = graph.begin_write();
    let a = txn
        .mutator()
        .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
        .expect("source node created");
    let b = txn
        .mutator()
        .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
        .expect("target node created");
    txn.mutator()
        .create_edge(istr("KNOWS"), a, b, PropertyMap::new())
        .expect("edge created");
    txn.commit().expect("seed commit succeeds");

    let values = row_values(execute(health_source(), &graph, &registry));

    assert_eq!(
        values,
        vec![
            Value::Uint(4102),
            Value::Uint(2),
            Value::Uint(1),
            Value::Bool(false),
        ]
    );
}

#[test]
fn selene_health_inside_explicit_tx_sees_uncommitted_inserts_via_real_registry() {
    let registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let graph = SharedGraph::new(GraphId::new(4103));
    let mut session = Session::new(&graph);

    execute_with_session("START TRANSACTION", &mut session, &registry);
    execute_with_session("INSERT (:Person)", &mut session, &registry);
    let values = row_values(execute_with_session(
        health_source(),
        &mut session,
        &registry,
    ));
    execute_with_session("ROLLBACK", &mut session, &registry);

    assert_eq!(
        values,
        vec![
            Value::Uint(4103),
            Value::Uint(1),
            Value::Uint(0),
            Value::Bool(false),
        ]
    );
    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn selene_health_rejects_runtime_arguments() {
    let runtime_registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let runtime_handle = runtime_health_handle(&runtime_registry);
    let planning_registry = PlanningRegistry::health_with_signature(
        runtime_handle,
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![ProcedureParameter::new(
            istr("unexpected"),
            GqlType::Integer,
            false,
        )],
    );
    let plan = planned("CALL selene.health(42) YIELD graph_id", &planning_registry);
    let graph = SharedGraph::new(GraphId::new(4104));
    let mut session = Session::new(&graph);

    let err = execute_statement(&plan, &mut session, &runtime_registry)
        .expect_err("runtime built-in rejects unexpected arg");

    assert!(matches!(
        err,
        selene_gql::ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn registry_execute_reports_tier_mismatch_expected_from_entry_actual_from_context() {
    let runtime_registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let runtime_handle = runtime_health_handle(&runtime_registry);
    let planning_registry = PlanningRegistry::health_with_signature(
        runtime_handle,
        ProcedureMutability::GraphWrite,
        ProcedureTier::Mutation,
        Vec::new(),
    );
    let plan = planned(health_source(), &planning_registry);
    let graph = SharedGraph::new(GraphId::new(4105));
    let mut session = Session::new(&graph);

    let err = execute_statement(&plan, &mut session, &runtime_registry)
        .expect_err("runtime registry entry is graph-tier");

    assert!(matches!(
        err,
        selene_gql::ExecutorError::Procedure {
            source: ProcedureError::TierMismatch {
                expected: ProcedureTier::Graph,
                actual: ProcedureTier::Mutation,
            },
            ..
        }
    ));
}

#[test]
fn top_level_call_yield_without_trailing_return_executes() {
    let registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let graph = SharedGraph::new(GraphId::new(4106));

    let output = execute(
        "CALL selene.health() YIELD graph_id, schema_bound",
        &graph,
        &registry,
    );

    let values = row_values(output);
    assert_eq!(values, vec![Value::Uint(4106), Value::Bool(false)]);
}

#[test]
fn top_level_call_with_trailing_return_stays_outside_phase_a_shape() {
    let err = parse("CALL selene.health() YIELD graph_id RETURN graph_id")
        .expect_err("top-level CALL does not accept trailing RETURN");

    assert!(!err.to_string().is_empty());
}

#[derive(Debug)]
struct PlanningRegistry {
    metadata: HashMap<Box<[IStr]>, ProcedureMetadata>,
}

impl PlanningRegistry {
    fn health_with_signature(
        handle: ProcedureHandle,
        mutability: ProcedureMutability,
        tier: ProcedureTier,
        parameters: Vec<ProcedureParameter>,
    ) -> Self {
        let mut metadata = HashMap::new();
        metadata.insert(
            vec![istr("selene"), istr("health")].into_boxed_slice(),
            ProcedureMetadata::new(
                handle,
                ProcedureSignature::new(parameters),
                ProcedureOutputSchema {
                    columns: vec![
                        output("graph_id", GqlType::Integer),
                        output("node_count", GqlType::Integer),
                        output("edge_count", GqlType::Integer),
                        output("schema_bound", GqlType::Boolean),
                    ],
                },
                tier,
                mutability,
                None,
            ),
        );
        Self { metadata }
    }
}

impl ProcedureRegistry for PlanningRegistry {
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        self.metadata.get(name).cloned()
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        unreachable!("planning-only registry is never executed")
    }
}

fn output(name: &str, ty: GqlType) -> ProcedureOutputColumn {
    ProcedureOutputColumn::new(istr(name), ty)
}

fn runtime_health_handle(registry: &ProcedurePackRegistry) -> ProcedureHandle {
    registry
        .lookup(&[istr("selene"), istr("health")])
        .expect("with_builtins() registers selene.health")
        .handle
}
