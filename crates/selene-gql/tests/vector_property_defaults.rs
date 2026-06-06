//! Vector property-default integration coverage.

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
            name: db_string("vector.defaults.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

#[test]
fn vector_default_materializes_and_round_trips_through_show() {
    let graph = empty_closed_graph(3910);
    let mut session = Session::new(&graph);

    execute(
        "CREATE NODE TYPE :Doc (embedding :: VECTOR DEFAULT [1, -0.0, 2.5])",
        &mut session,
    )
    .expect("catalog succeeds");
    execute("INSERT (n:Doc) FINISH", &mut session).expect("insert succeeds");
    let table = rows(
        execute(
            "MATCH (n:Doc) RETURN n.embedding AS embedding",
            &mut session,
        )
        .expect("match succeeds"),
    );

    assert_eq!(
        table.rows()[0].values(),
        &[Value::Vector(
            VectorValue::new(vec![1.0, 0.0, 2.5]).unwrap()
        )]
    );

    let table = rows(execute("SHOW NODE TYPES", &mut session).expect("show succeeds"));
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Doc (embedding :: VECTOR DEFAULT [1.0, 0.0, 2.5])"
        ))
    );
}
