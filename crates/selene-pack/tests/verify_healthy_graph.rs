//! `selene.verify` healthy-graph integration tests.

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{BindingTable, ProcedureRegistry, Session, StatementOutput};
use selene_graph::{SharedGraph, TypedIndexKind};
use selene_pack::ProcedurePackRegistry;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn execute_rows(
    session: &mut Session<'_>,
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> BindingTable {
    rows(
        session
            .execute_source(source, registry)
            .expect("statement executes"),
    )
}

fn column_strings(table: &BindingTable, name: &str) -> Vec<String> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::String(value)) => value.as_str().to_owned(),
            Some(Value::ExternalString(value)) => value.as_ref().to_owned(),
            other => panic!("expected string in {name}, got {other:?}"),
        })
        .collect()
}

fn healthy_graph() -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(121_501));
    let person = istr("Person");
    let knows = istr("KNOWS");
    let age = istr("age");
    let mut alice_props = PropertyMap::new();
    alice_props.set(age, Value::Int(41)).unwrap();
    let mut bob_props = PropertyMap::new();
    bob_props.set(age, Value::Int(42)).unwrap();

    let mut txn = graph.begin_write();
    let alice = txn
        .mutator()
        .create_node(LabelSet::single(person), alice_props)
        .expect("alice created");
    let bob = txn
        .mutator()
        .create_node(LabelSet::single(person), bob_props)
        .expect("bob created");
    txn.mutator()
        .create_edge(knows, alice, bob, PropertyMap::new())
        .expect("edge created");
    txn.mutator()
        .create_property_index(person, age, TypedIndexKind::I64)
        .expect("index created");
    txn.commit().expect("seed commit succeeds");
    graph
}

#[test]
fn selene_verify_reports_ok_for_healthy_graph() {
    let registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let graph = healthy_graph();
    let mut session = Session::new(&graph);

    let table = execute_rows(
        &mut session,
        "CALL selene.verify() YIELD check, status, detail",
        &registry,
    );
    let checks = column_strings(&table, "check");
    let statuses = column_strings(&table, "status");

    assert_eq!(
        checks,
        vec![
            "label_index_cardinality",
            "property_index_coverage",
            "adjacency_symmetry",
            "edge_endpoint_liveness",
        ]
    );
    assert_eq!(statuses, vec!["ok", "ok", "ok", "ok"]);
}

#[test]
fn show_procedures_lists_selene_verify() {
    let registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let graph = SharedGraph::new(GraphId::new(121_502));
    let mut session = Session::new(&graph);

    let table = execute_rows(&mut session, "SHOW PROCEDURES", &registry);
    let names = column_strings(&table, "name");

    assert!(names.contains(&"selene.verify".to_owned()));
}
