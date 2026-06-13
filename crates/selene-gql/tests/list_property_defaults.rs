//! LIST property-default integration coverage.

use selene_core::{GraphId, Value, VectorValue};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ExecutorError, Session, StatementOutput, analyze,
    execute_statement, parse, plan,
};
use selene_graph::{GraphTypeDef, SharedGraph};

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
            name: db_string("list.defaults.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

#[test]
fn list_defaults_materialize_and_round_trip_through_show() {
    let graph = empty_closed_graph(3920);
    let mut session = Session::new(&graph);

    execute(
        "CREATE NODE TYPE :Doc (\
         tags :: LIST<STRING> DEFAULT ['alpha', 'beta'], \
         matrix :: LIST<LIST<INTEGER>> DEFAULT [[1, 2], [3]], \
         embeddings :: LIST<VECTOR> DEFAULT [[1, 0], [0, 1]])",
        &mut session,
    )
    .expect("catalog succeeds");
    execute("INSERT (n:Doc) FINISH", &mut session).expect("insert succeeds");
    let table = rows(
        execute(
            "MATCH (n:Doc) RETURN n.tags AS tags, n.matrix AS matrix, n.embeddings AS embeddings",
            &mut session,
        )
        .expect("match succeeds"),
    );

    assert_eq!(
        table.rows()[0].values(),
        &[
            Value::List(vec![
                Value::String(db_string("alpha")),
                Value::String(db_string("beta")),
            ]),
            Value::List(vec![
                Value::List(vec![Value::Int(1), Value::Int(2)]),
                Value::List(vec![Value::Int(3)]),
            ]),
            Value::List(vec![
                Value::Vector(VectorValue::new(vec![1.0, 0.0]).unwrap()),
                Value::Vector(VectorValue::new(vec![0.0, 1.0]).unwrap()),
            ]),
        ]
    );

    let table = rows(execute("SHOW NODE TYPES", &mut session).expect("show succeeds"));
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Doc (tags :: LIST<STRING> DEFAULT ['alpha', 'beta'], \
             matrix :: LIST<LIST<INTEGER>> DEFAULT [[1, 2], [3]], \
             embeddings :: LIST<VECTOR> DEFAULT [[1.0D, 0.0D], [0.0D, 1.0D]])"
        ))
    );
}
