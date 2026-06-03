//! BRIEF-39 procedure CALL pipeline executor tests.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutionPlan,
    ExecutorError, GqlStatus, GqlType, PipelineOp, ProcedureContext, ProcedureError,
    ProcedureHandle, ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn,
    ProcedureOutputSchema, ProcedureParameter, ProcedureRegistry, ProcedureResult,
    ProcedureSignature, ProcedureTier, Session, StatementOutput, TxContext, analyze,
    execute_pipeline, execute_statement, parse, plan,
};
use selene_graph::SharedGraph;

#[derive(Clone, Debug)]
enum Behavior {
    Return(Vec<Vec<Value>>),
    CountNodes,
    CreateNode(IStr),
    Error(ProcedureError),
}

#[derive(Clone, Debug, PartialEq)]
struct CallRecord {
    handle: ProcedureHandle,
    args: Vec<Value>,
    tier: ProcedureTier,
}

#[derive(Debug, Default)]
struct TestRegistry {
    metadata: HashMap<Box<[IStr]>, ProcedureMetadata>,
    behavior: HashMap<u64, Behavior>,
    records: Mutex<Vec<CallRecord>>,
    next_handle: u64,
}

impl TestRegistry {
    fn new() -> Self {
        Self {
            metadata: HashMap::new(),
            behavior: HashMap::new(),
            records: Mutex::new(Vec::new()),
            next_handle: 1,
        }
    }

    fn with_procedure(
        mut self,
        name: &[&str],
        mutability: ProcedureMutability,
        tier: ProcedureTier,
        parameters: Vec<ProcedureParameter>,
        outputs: Vec<ProcedureOutputColumn>,
        behavior: Behavior,
    ) -> Self {
        let handle = ProcedureHandle::new(self.next_handle);
        self.next_handle += 1;
        self.behavior.insert(handle.raw(), behavior);
        self.metadata.insert(
            name.iter()
                .map(|segment| istr(segment))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            ProcedureMetadata::new(
                handle,
                ProcedureSignature::new(parameters),
                ProcedureOutputSchema { columns: outputs },
                tier,
                mutability,
            ),
        );
        self
    }

    fn records(&self) -> MutexGuard<'_, Vec<CallRecord>> {
        self.records.lock().expect("records mutex")
    }
}

impl ProcedureRegistry for TestRegistry {
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        self.metadata.get(name).cloned()
    }

    fn execute(
        &self,
        handle: ProcedureHandle,
        args: &[Value],
        ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        self.records().push(CallRecord {
            handle,
            args: args.to_vec(),
            tier: ctx.tier(),
        });

        match self
            .behavior
            .get(&handle.raw())
            .expect("planned handle has behavior")
        {
            Behavior::Return(rows) => Ok(procedure_result(rows.clone())),
            Behavior::CountNodes => Ok(procedure_result(vec![vec![Value::Int(match ctx {
                ProcedureContext::Graph(graph) => graph.snapshot().node_count() as i64,
                ProcedureContext::Mutation(mutation) => mutation.snapshot().node_count() as i64,
                _ => unreachable!("unknown procedure context"),
            })]])),
            Behavior::CreateNode(label) => {
                let ProcedureContext::Mutation(mutation) = ctx else {
                    return Err(ProcedureError::Internal {
                        detail: "create node behavior requires mutation context".to_owned(),
                    });
                };
                mutation
                    .mutator()
                    .create_node(LabelSet::single(label.clone()), PropertyMap::new())
                    .map_err(|source| ProcedureError::Internal {
                        detail: source.to_string(),
                    })?;
                Ok(procedure_result(vec![vec![]]))
            }
            Behavior::Error(error) => Err(error.clone()),
        }
    }
}

fn procedure_result(rows: Vec<Vec<Value>>) -> ProcedureResult {
    ProcedureResult { rows }
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn param(name: &str, ty: GqlType, nullable: bool) -> ProcedureParameter {
    ProcedureParameter::new(istr(name), ty, nullable)
}

fn output(name: &str, ty: GqlType) -> ProcedureOutputColumn {
    ProcedureOutputColumn::new(istr(name), ty)
}

fn registry_one(
    name: &[&str],
    mutability: ProcedureMutability,
    tier: ProcedureTier,
    outputs: Vec<ProcedureOutputColumn>,
    behavior: Behavior,
) -> TestRegistry {
    TestRegistry::new().with_procedure(name, mutability, tier, Vec::new(), outputs, behavior)
}

fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, registry, None).expect("test input analyzes");
    plan(&analyzed, registry).expect("test input plans")
}

fn execute(
    source: &str,
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let plan = planned(source, registry);
    let mut session = Session::new(graph);
    execute_statement(&plan, &mut session, registry)
}

fn execute_with_session(
    source: &str,
    session: &mut Session<'_>,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let plan = planned(source, registry);
    execute_statement(&plan, session, registry)
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn seed_table() -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: Vec::new(),
        },
        vec![Binding::empty()],
    )
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn column_values(table: &BindingTable, target: &str) -> Vec<Value> {
    let index = table
        .schema()
        .columns
        .iter()
        .position(|column| {
            column
                .name
                .clone()
                .is_some_and(|name| name.as_str() == target)
        })
        .expect("column exists");
    table
        .rows()
        .iter()
        .map(|row| row.values()[index].clone())
        .collect()
}

#[test]
fn unknown_procedure_at_runtime_maps_to_unknown_procedure_status() {
    let registry = registry_one(
        &["pkg", "rows"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(1)]]),
    );
    let plan = planned("CALL pkg.rows() YIELD out", &registry);
    let graph = graph(3900);
    let mut session = Session::new(&graph);

    let err = execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect_err("runtime registry misses handle");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::UnknownProcedure { .. },
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::UNKNOWN_PROCEDURE);
}

#[test]
fn read_tier_procedure_yields_rows() {
    let registry = registry_one(
        &["pkg", "rows"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(7)]]),
    );

    let table = rows(execute("CALL pkg.rows() YIELD out", &graph(3901), &registry).unwrap());

    assert_eq!(column_values(&table, "out"), vec![Value::Int(7)]);
    assert_eq!(registry.records()[0].tier, ProcedureTier::Graph);
}

#[test]
fn procedure_returning_zero_rows_drops_input_row() {
    let registry = registry_one(
        &["pkg", "empty"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(Vec::new()),
    );
    let plan = planned("CALL pkg.empty() YIELD out", &registry);
    let graph = graph(3902);
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &registry,
        graph.index_providers(),
    );

    let table = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).unwrap();

    assert_eq!(table.row_count(), 0);
    assert_eq!(
        table.schema().columns[0].name.clone().unwrap().as_str(),
        "out"
    );
}

#[test]
fn procedure_returning_one_unit_row_emits_one_row_per_input() {
    let registry = registry_one(
        &["pkg", "unit"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );
    let plan = planned("CALL pkg.unit()", &registry);
    let graph = graph(3903);
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &registry,
        graph.index_providers(),
    );

    let table = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).unwrap();

    assert_eq!(table.row_count(), 1);
    assert!(table.schema().columns.is_empty());
    assert!(table.rows()[0].values().is_empty());
}

#[test]
fn read_tier_procedure_cross_products_with_multi_row_input() {
    let registry = registry_one(
        &["pkg", "two"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("y", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(10)], vec![Value::Int(20)]]),
    );

    let table = rows(
        execute(
            "UNWIND [1, 2, 3] AS x CALL pkg.two() YIELD y RETURN x, y",
            &graph(3904),
            &registry,
        )
        .unwrap(),
    );

    assert_eq!(
        column_values(&table, "x"),
        vec![
            Value::Int(1),
            Value::Int(1),
            Value::Int(2),
            Value::Int(2),
            Value::Int(3),
            Value::Int(3)
        ]
    );
    assert_eq!(
        column_values(&table, "y"),
        vec![
            Value::Int(10),
            Value::Int(20),
            Value::Int(10),
            Value::Int(20),
            Value::Int(10),
            Value::Int(20)
        ]
    );
    assert_eq!(registry.records().len(), 3);
}

#[test]
fn read_tier_procedure_yield_named_selects_columns_by_name() {
    let registry = registry_one(
        &["pkg", "values"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![
            output("a", GqlType::Integer),
            output("b", GqlType::Integer),
            output("c", GqlType::Integer),
        ],
        Behavior::Return(vec![vec![Value::Int(1), Value::Int(2), Value::Int(3)]]),
    );

    let table = rows(execute("CALL pkg.values() YIELD c, a", &graph(3905), &registry).unwrap());

    assert_eq!(table.rows()[0].values(), &[Value::Int(3), Value::Int(1)]);
    let names = table
        .schema()
        .columns
        .iter()
        .map(|column| column.name.as_ref().unwrap().as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["c", "a"]);
}

#[test]
fn read_tier_procedure_yield_star_emits_all_columns_in_schema_order() {
    let registry = registry_one(
        &["pkg", "values"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("a", GqlType::Integer), output("b", GqlType::String)],
        Behavior::Return(vec![vec![Value::Int(1), Value::String(istr("two"))]]),
    );

    let table = rows(execute("CALL pkg.values() YIELD *", &graph(3906), &registry).unwrap());

    assert_eq!(
        table.rows()[0].values(),
        &[Value::Int(1), Value::String(istr("two"))]
    );
}

#[test]
fn read_tier_procedure_inside_write_tx_sees_pending_writes() {
    let registry = registry_one(
        &["pkg", "count"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("count", GqlType::Integer)],
        Behavior::CountNodes,
    );
    let graph = graph(3907);
    let mut session = Session::new(&graph);

    execute_with_session("START TRANSACTION", &mut session, &registry).unwrap();
    execute_with_session("INSERT (n:Person) FINISH", &mut session, &registry).unwrap();
    let table = rows(
        execute_with_session("CALL pkg.count() YIELD count", &mut session, &registry).unwrap(),
    );

    assert_eq!(column_values(&table, "count"), vec![Value::Int(1)]);
    assert_eq!(graph.read().node_count(), 0);
    session.abort();
}

#[test]
fn mutation_tier_procedure_in_auto_commit_commits_on_success() {
    let registry = registry_one(
        &["pkg", "create"],
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        Vec::new(),
        Behavior::CreateNode(istr("FromProc")),
    );
    let graph = graph(3908);

    let output = execute("CALL pkg.create()", &graph, &registry).unwrap();

    assert!(matches!(
        output,
        StatementOutput::Written(outcome) if outcome.changes.len() == 1
    ));
    assert_eq!(graph.read().node_count(), 1);
    assert_eq!(registry.records()[0].tier, ProcedureTier::Mutation);
}

#[test]
fn maintenance_tier_procedure_runs_without_write_commit() {
    let registry = registry_one(
        &["pkg", "maintain"],
        ProcedureMutability::MaintenanceWrite,
        ProcedureTier::Maintenance,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(11)]]),
    );
    let graph = graph(3926);

    let table = rows(execute("CALL pkg.maintain() YIELD out", &graph, &registry).unwrap());

    assert_eq!(column_values(&table, "out"), vec![Value::Int(11)]);
    assert_eq!(graph.read().node_count(), 0);
    assert_eq!(registry.records()[0].tier, ProcedureTier::Maintenance);
}

#[test]
fn maintenance_tier_procedure_in_explicit_tx_is_rejected() {
    let registry = registry_one(
        &["pkg", "maintain"],
        ProcedureMutability::MaintenanceWrite,
        ProcedureTier::Maintenance,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );
    let graph = graph(3927);
    let mut session = Session::new(&graph);

    execute_with_session("START TRANSACTION", &mut session, &registry).unwrap();
    let err = execute_with_session("CALL pkg.maintain()", &mut session, &registry)
        .expect_err("maintenance rejects explicit transaction");

    assert!(matches!(
        err,
        ExecutorError::InvalidTransactionState {
            detail: "maintenance procedure cannot run inside an explicit transaction",
            ..
        }
    ));
    assert!(registry.records().is_empty());
    assert!(session.is_aborted());
    session.abort();
}

#[test]
fn maintenance_tier_procedure_in_plain_read_context_is_rejected() {
    let registry = registry_one(
        &["pkg", "maintain"],
        ProcedureMutability::MaintenanceWrite,
        ProcedureTier::Maintenance,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );
    let plan = planned("CALL pkg.maintain()", &registry);
    let graph = graph(3928);
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &registry,
        graph.index_providers(),
    );

    let err = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx)
        .expect_err("maintenance context is absent");

    assert!(matches!(
        err,
        ExecutorError::InvalidTransactionState {
            detail: "maintenance-tier procedure requires a maintenance statement context",
            ..
        }
    ));
}

#[test]
fn mutation_tier_procedure_in_explicit_tx_sees_pending_state() {
    let registry = registry_one(
        &["pkg", "count"],
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        vec![output("count", GqlType::Integer)],
        Behavior::CountNodes,
    );
    let graph = graph(3909);
    let mut session = Session::new(&graph);

    execute_with_session("START TRANSACTION", &mut session, &registry).unwrap();
    execute_with_session("INSERT (n:Person) FINISH", &mut session, &registry).unwrap();
    let table = rows(
        execute_with_session("CALL pkg.count() YIELD count", &mut session, &registry).unwrap(),
    );

    assert_eq!(column_values(&table, "count"), vec![Value::Int(1)]);
    assert_eq!(graph.read().node_count(), 0);
    session.abort();
}

#[test]
fn mutation_tier_procedure_in_read_only_context_returns_invalid_transaction_state() {
    let registry = registry_one(
        &["pkg", "create"],
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        Vec::new(),
        Behavior::CreateNode(istr("FromProc")),
    );
    let plan = planned("CALL pkg.create()", &registry);
    let graph = graph(3910);
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &registry,
        graph.index_providers(),
    );

    let err =
        execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).expect_err("needs write txn");

    assert!(matches!(
        err,
        ExecutorError::InvalidTransactionState {
            detail: "mutation-tier procedure requires a write transaction",
            ..
        }
    ));
}

#[test]
fn mutation_tier_procedure_failure_inside_explicit_tx_marks_session_aborted() {
    let registry = registry_one(
        &["pkg", "fail"],
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        Vec::new(),
        Behavior::Error(ProcedureError::InvalidArgument {
            detail: "bad input".to_owned(),
        }),
    );
    let graph = graph(3911);
    let mut session = Session::new(&graph);

    execute_with_session("START TRANSACTION", &mut session, &registry).unwrap();
    let err = execute_with_session("CALL pkg.fail()", &mut session, &registry)
        .expect_err("procedure fails");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
    assert!(session.is_aborted());
    session.abort();
}

#[test]
fn tier_mismatch_between_metadata_and_dispatch_returns_tier_mismatch() {
    let registry = registry_one(
        &["pkg", "bad"],
        ProcedureMutability::Read,
        ProcedureTier::Mutation,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );

    let err = execute("CALL pkg.bad()", &graph(3912), &registry).expect_err("tier mismatch");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::TierMismatch {
                expected: ProcedureTier::Graph,
                actual: ProcedureTier::Mutation
            },
            ..
        }
    ));
}

#[test]
fn procedure_returns_wrong_column_count_returns_internal_error() {
    let registry = registry_one(
        &["pkg", "bad"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![]]),
    );

    let err = execute("CALL pkg.bad() YIELD out", &graph(3913), &registry)
        .expect_err("row width mismatch");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::Internal { detail },
            ..
        } if detail == "registry returned row with wrong column count"
    ));
}

#[test]
fn procedure_returns_wrong_value_type_returns_internal_error() {
    let registry = registry_one(
        &["pkg", "bad"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::String(istr("wrong"))]]),
    );

    let err = execute("CALL pkg.bad() YIELD out", &graph(3914), &registry)
        .expect_err("row type mismatch");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::Internal { detail },
            ..
        } if detail == "registry returned value with wrong type for column 0"
    ));
}

#[test]
fn procedure_timeout_preserves_session_deadline() {
    let elapsed = Duration::from_millis(7);
    let registry = registry_one(
        &["pkg", "timeout"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        Vec::new(),
        Behavior::Error(ProcedureError::Timeout { elapsed }),
    );
    let graph = graph(3915);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut session = Session::new(&graph).with_deadline(deadline);

    let err = execute_with_session("CALL pkg.timeout()", &mut session, &registry)
        .expect_err("procedure reports timeout");

    let ExecutorError::Timeout {
        deadline: observed,
        elapsed: observed_elapsed,
        ..
    } = err
    else {
        panic!("expected timeout, got {err:?}");
    };
    assert_eq!(observed, deadline);
    assert_eq!(observed_elapsed, elapsed);
}

#[test]
fn arg_evaluation_failure_propagates_without_dispatch() {
    let registry = TestRegistry::new().with_procedure(
        &["pkg", "echo"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![param("value", GqlType::Integer, false)],
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(1)]]),
    );

    let err = execute("CALL pkg.echo(1 / 0) YIELD out", &graph(3916), &registry)
        .expect_err("arg evaluation fails");

    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert!(registry.records().is_empty());
}

#[test]
fn procedure_with_zero_args_dispatches_with_empty_arg_slice() {
    let registry = registry_one(
        &["pkg", "unit"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );

    execute("CALL pkg.unit()", &graph(3917), &registry).unwrap();

    assert_eq!(registry.records()[0].args, Vec::<Value>::new());
}

#[test]
fn call_after_anonymous_insert_preserves_per_row_insert_sites() {
    let registry = registry_one(
        &["pkg", "unit"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );
    let mut plan = planned("INSERT (:A)-[:E]->(:B) FINISH", &registry);
    let call = planned("CALL pkg.unit()", &registry)
        .pipeline
        .into_iter()
        .next()
        .expect("call op");
    let edge_index = plan
        .pipeline
        .iter()
        .position(|op| {
            matches!(
                op,
                PipelineOp::Mutation(selene_gql::MutationOp::InsertEdge { .. })
            )
        })
        .expect("edge insert op");
    plan.pipeline.insert(edge_index, call);
    let graph = graph(3918);
    let mut session = Session::new(&graph);

    execute_statement(&plan, &mut session, &registry).expect("insert chain executes");

    let snapshot = graph.read();
    assert_eq!(snapshot.node_count(), 2);
    assert_eq!(snapshot.edge_count(), 1);
}
