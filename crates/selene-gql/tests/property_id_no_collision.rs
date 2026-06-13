//! Regression guard for property `id` vs. element identity lookups.

use selene_core::{DbString, GraphId, Value};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::{GraphTypeDef, SharedGraph};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

#[test]
fn node_property_named_id_wins_property_access() {
    let graph = SharedGraph::builder(GraphId::new(128_101))
        .bound_to(GraphTypeDef {
            name: db_string("id.collision.graph"),
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
    assert_eq!(table.rows()[0].values(), &[Value::String(db_string("abc"))]);
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}
