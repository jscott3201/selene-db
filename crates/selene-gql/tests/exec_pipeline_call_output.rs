//! Procedure CALL output type validation regressions.

use std::collections::HashMap;

use selene_core::{DbString, GraphId, Record, Value};
use selene_gql::{
    BindingTable, ExecutionPlan, ExecutorError, GqlType, ProcedureContext, ProcedureError,
    ProcedureHandle, ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn,
    ProcedureOutputSchema, ProcedureRegistry, ProcedureResult, ProcedureSignature, ProcedureTier,
    RecordType, Session, StatementOutput, analyze, execute_statement, parse, plan,
};
use selene_graph::SharedGraph;
use smallvec::smallvec;

#[derive(Debug)]
struct OutputRegistry {
    metadata: HashMap<Box<[DbString]>, ProcedureMetadata>,
    rows: Vec<Vec<Value>>,
}

impl OutputRegistry {
    fn new(outputs: Vec<ProcedureOutputColumn>, rows: Vec<Vec<Value>>) -> Self {
        let handle = ProcedureHandle::new(1);
        let mut metadata = HashMap::new();
        metadata.insert(
            [db_string("pkg"), db_string("out")]
                .into_iter()
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            ProcedureMetadata::new(
                handle,
                ProcedureSignature::new(Vec::new()),
                ProcedureOutputSchema { columns: outputs },
                ProcedureTier::Graph,
                ProcedureMutability::Read,
            ),
        );
        Self { metadata, rows }
    }
}

impl ProcedureRegistry for OutputRegistry {
    fn lookup(&self, name: &[DbString]) -> Option<ProcedureMetadata> {
        self.metadata.get(name).cloned()
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        Ok(ProcedureResult {
            rows: self.rows.clone(),
        })
    }
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn output(name: &str, ty: GqlType) -> ProcedureOutputColumn {
    ProcedureOutputColumn::new(db_string(name), ty)
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

fn graph() -> SharedGraph {
    SharedGraph::new(GraphId::new(5010))
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
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

fn assert_wrong_output_type(error: ExecutorError) {
    assert!(matches!(
        error,
        ExecutorError::Procedure {
            source: ProcedureError::Internal { detail },
            ..
        } if detail == "registry returned value with wrong type for column 0"
    ));
}

#[test]
fn procedure_output_int8_rejects_out_of_range_int() {
    let registry = OutputRegistry::new(
        vec![output("out", GqlType::Int8)],
        vec![vec![Value::Int(128)]],
    );

    let err = execute("CALL pkg.out() YIELD out", &graph(), &registry).expect_err("int8 overflow");

    assert_wrong_output_type(err);
}

#[test]
fn procedure_output_generic_float_accepts_float32() {
    let registry = OutputRegistry::new(
        vec![output("out", GqlType::Float)],
        vec![vec![Value::Float32(1.25)]],
    );

    let table = rows(execute("CALL pkg.out() YIELD out", &graph(), &registry).expect("float32"));

    assert_eq!(column_values(&table, "out"), vec![Value::Float32(1.25)]);
}

#[test]
fn procedure_output_closed_record_rejects_extra_fields() {
    let expected_field = db_string("count");
    let extra_field = db_string("name");
    let returned = Value::Record(Box::new(Record::Open(smallvec![
        (expected_field.clone(), Value::Int(3)),
        (extra_field, Value::String(db_string("extra"))),
    ])));
    let registry = OutputRegistry::new(
        vec![output(
            "out",
            GqlType::Record(RecordType::Closed(vec![(expected_field, GqlType::Integer)])),
        )],
        vec![vec![returned]],
    );

    let err =
        execute("CALL pkg.out() YIELD out", &graph(), &registry).expect_err("extra record field");

    assert_wrong_output_type(err);
}

#[test]
fn procedure_output_null_remains_allowed_for_declared_columns() {
    let registry = OutputRegistry::new(
        vec![output("out", GqlType::String)],
        vec![vec![Value::Null]],
    );

    let table = rows(execute("CALL pkg.out() YIELD out", &graph(), &registry).expect("null"));

    assert_eq!(column_values(&table, "out"), vec![Value::Null]);
}
