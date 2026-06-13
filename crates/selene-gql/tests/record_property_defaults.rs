//! RECORD property-default integration coverage.

use selene_core::{GraphId, Record, Value};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ExecutorError, Session, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::{GraphTypeDef, SharedGraph};
use smallvec::smallvec;

fn db_string(value: &str) -> selene_core::DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn execute(source: &str, session: &mut Session<'_>) -> Result<StatementOutput, ExecutorError> {
    let plan = planned(source);
    execute_statement(&plan, session, &EmptyProcedureRegistry)
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("record.defaults.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

#[test]
fn record_defaults_materialize_and_round_trip_through_show() {
    let graph = empty_closed_graph(3930);
    let mut session = Session::new(&graph);

    execute(
        "CREATE NODE TYPE :Host (config :: RECORD{\
         host :: STRING, \
         port :: INTEGER, \
         nested :: RECORD{flag :: BOOLEAN}, \
         tags :: LIST<STRING>} \
         DEFAULT RECORD{host: 'h', port: 1, nested: RECORD{flag: true}, tags: ['agent', 'memory']})",
        &mut session,
    )
    .expect("catalog succeeds");
    execute("INSERT (n:Host) FINISH", &mut session).expect("insert succeeds");
    let table = rows(
        execute("MATCH (n:Host) RETURN n.config AS config", &mut session).expect("match succeeds"),
    );

    assert_eq!(
        table.rows()[0].values(),
        &[Value::Record(Box::new(Record::Open(smallvec![
            (db_string("host"), Value::String(db_string("h"))),
            (db_string("port"), Value::Int(1)),
            (
                db_string("nested"),
                Value::Record(Box::new(Record::Open(smallvec![(
                    db_string("flag"),
                    Value::Bool(true)
                )])))
            ),
            (
                db_string("tags"),
                Value::List(vec![
                    Value::String(db_string("agent")),
                    Value::String(db_string("memory")),
                ])
            ),
        ])))]
    );

    let table = rows(execute("SHOW NODE TYPES", &mut session).expect("show succeeds"));
    let Value::String(definition) = &table.rows()[0].values()[1] else {
        panic!("definition is a string");
    };
    assert_eq!(
        definition.as_str(),
        "CREATE NODE TYPE :Host (config :: RECORD { host :: STRING, port :: INTEGER, \
         nested :: RECORD { flag :: BOOLEAN }, tags :: LIST<STRING> } DEFAULT \
         RECORD{host: 'h', port: 1, nested: RECORD{flag: TRUE}, tags: ['agent', 'memory']})"
    );
    parse(definition.as_str()).expect("record default definition round-trips through parser");
}
