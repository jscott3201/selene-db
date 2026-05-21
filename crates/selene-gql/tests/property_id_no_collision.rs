//! Regression guard for property `id` vs. element identity lookups.

use selene_core::{GraphId, IStr, Value, intern};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::{GraphTypeDef, SharedGraph};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

#[test]
fn node_property_named_id_wins_property_access() {
    let graph = SharedGraph::builder(GraphId::new(128_101))
        .bound_to(GraphTypeDef {
            name: istr("id.collision.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .expect("graph type binds")
        .build()
        .expect("closed graph builds");
    let registry = EmptyProcedureRegistry;
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CREATE NODE TYPE :Sensor (id :: STRING NOT NULL)",
            &registry,
        )
        .expect("type DDL executes");
    session
        .execute_source("INSERT (n:Sensor {id: 'abc'}) FINISH", &registry)
        .expect("sensor inserts");
    let table = rows(
        session
            .execute_source(
                "MATCH (n:Sensor) WHERE n.id = 'abc' RETURN n.id AS id",
                &registry,
            )
            .expect("query executes"),
    );

    assert_eq!(table.row_count(), 1);
    assert_eq!(table.rows()[0].values(), &[Value::String(istr("abc"))]);
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}
