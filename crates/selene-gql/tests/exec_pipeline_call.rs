//! BRIEF-39 procedure CALL pipeline executor tests.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use selene_core::{DbString, GraphId, LabelSet, PropertyMap, Value};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, ExecutionPlan, ExecutorError, GqlType,
    ProcedureContext, ProcedureError, ProcedureHandle, ProcedureMetadata, ProcedureMutability,
    ProcedureOutputColumn, ProcedureOutputSchema, ProcedureParameter, ProcedureRegistry,
    ProcedureResult, ProcedureSignature, ProcedureTier, Session, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::SharedGraph;

#[derive(Clone, Debug)]
enum Behavior {
    Return(Vec<Vec<Value>>),
    CountNodes,
    CreateNode(DbString),
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
    metadata: HashMap<Box<[DbString]>, ProcedureMetadata>,
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
                .map(|segment| db_string(segment))
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
    fn lookup(&self, name: &[DbString]) -> Option<ProcedureMetadata> {
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

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn param(name: &str, ty: GqlType, nullable: bool) -> ProcedureParameter {
    ProcedureParameter::new(db_string(name), ty, nullable)
}

fn output(name: &str, ty: GqlType) -> ProcedureOutputColumn {
    ProcedureOutputColumn::new(db_string(name), ty)
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

// Test subdomains live in sibling files to keep this test root under the
// 700-LOC cap; they reuse this binary's TestRegistry harness and helpers.
#[path = "exec_pipeline_call/tier_dispatch.rs"]
mod tier_dispatch;
#[path = "exec_pipeline_call/yield_and_shape.rs"]
mod yield_and_shape;
