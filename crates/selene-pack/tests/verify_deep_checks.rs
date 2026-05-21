//! `selene.verify` deep-check integration tests.

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{BindingTable, ProcedureRegistry, Session, StatementOutput};
use selene_graph::{SharedGraph, TypedIndexKind};
use selene_pack::{ProcedureDefaultValue, ProcedurePackRegistry};

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

fn indexed_graph() -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(121_601));
    let person = istr("Person");
    let age = istr("age");
    let mut props = PropertyMap::new();
    props.set(age, Value::Int(42)).unwrap();

    let mut txn = graph.begin_write();
    txn.mutator()
        .create_node(LabelSet::single(person), props)
        .expect("node created");
    txn.mutator()
        .create_property_index(person, age, TypedIndexKind::I64)
        .expect("index created");
    txn.commit().expect("seed commit succeeds");
    graph
}

#[test]
fn selene_verify_omits_deep_rows_by_default() {
    let registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let graph = indexed_graph();
    let mut session = Session::new(&graph);

    let table = execute_rows(
        &mut session,
        "CALL selene.verify() YIELD check, status, detail",
        &registry,
    );
    let checks = column_strings(&table, "check");

    assert!(!checks.contains(&"typed_index_value_range".to_owned()));
    assert!(!checks.contains(&"roaring_bitmap_density".to_owned()));
}

#[test]
fn selene_verify_deep_true_emits_deep_rows() {
    let registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let graph = indexed_graph();
    let mut session = Session::new(&graph);

    let table = execute_rows(
        &mut session,
        "CALL selene.verify(TRUE) YIELD check, status, detail",
        &registry,
    );
    let checks = column_strings(&table, "check");
    let statuses = column_strings(&table, "status");

    assert!(checks.contains(&"typed_index_value_range".to_owned()));
    assert!(checks.contains(&"roaring_bitmap_density".to_owned()));
    assert!(statuses.iter().all(|status| status == "ok"));
}

#[test]
fn selene_verify_metadata_declares_optional_deep_default() {
    let registry = ProcedurePackRegistry::with_builtins()
        .expect("platform built-ins register cleanly in tests");
    let metadata = registry
        .lookup(&[istr("selene"), istr("verify")])
        .expect("selene.verify is registered");

    assert_eq!(metadata.signature.parameters.len(), 1);
    let deep = &metadata.signature.parameters[0];
    assert_eq!(deep.name.as_str(), "deep");
    assert_eq!(deep.default, Some(ProcedureDefaultValue::Boolean(false)));
}
