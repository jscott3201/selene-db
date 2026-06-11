//! Seed-bound scan short-circuit regression tests.

mod exec_common;
#[path = "seeded_scan/oracle.rs"]
mod oracle;

use exec_common::{ExecFixture, column_values, db_string, execute_plan, optimized, props};
use selene_core::Value;
use selene_gql::{EmptyProcedureRegistry, ExecutorError, Session, StatementOutput};
use selene_graph::{SharedGraph, TypedIndexKind};

fn execute_optimized(source: &str) -> selene_gql::BindingTable {
    let fixture = ExecFixture::build();
    let plan = optimized(source, &fixture.index_catalog());
    execute_plan(&fixture, &plan).expect("optimized query executes")
}

fn execute_on_graph(
    graph: &SharedGraph,
    source: &str,
) -> Result<selene_gql::BindingTable, ExecutorError> {
    let mut session = Session::new(graph);
    match session.execute_source(source, &EmptyProcedureRegistry)? {
        StatementOutput::Rows(table) => Ok(table),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn strings(table: &selene_gql::BindingTable, name: &str) -> Vec<String> {
    column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::String(value) => value.as_str().to_owned(),
            other => panic!("expected string, got {other:?}"),
        })
        .collect()
}

fn ints(table: &selene_gql::BindingTable, name: &str) -> Vec<i64> {
    column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::Int(value) => value,
            other => panic!("expected integer, got {other:?}"),
        })
        .collect()
}

fn bools(table: &selene_gql::BindingTable, name: &str) -> Vec<bool> {
    column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::Bool(value) => value,
            other => panic!("expected boolean, got {other:?}"),
        })
        .collect()
}

#[test]
fn seeded_linear_non_leading_match_reuses_outer_node() {
    let table = execute_optimized(
        "MATCH (a:Person)
         MATCH (a)
         RETURN a.name AS name
         ORDER BY name",
    );

    assert_eq!(
        strings(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
}

#[test]
fn seeded_label_mismatch_returns_empty() {
    let table = execute_optimized(
        "MATCH (a:Person)
         MATCH (a:Sensor)
         RETURN a.name AS name
         ORDER BY name",
    );

    assert_eq!(table.row_count(), 0);
}

#[test]
fn seeded_typed_range_reapplies_value_constraint() {
    let table = execute_optimized(
        "MATCH (a:Person)
         MATCH (a:Person)
         WHERE a.age >= 40
         RETURN a.name AS name
         ORDER BY name",
    );

    assert_eq!(
        strings(&table, "name"),
        vec!["Bob".to_owned(), "Cara".to_owned()]
    );
}

#[test]
fn seeded_bitmap_union_reapplies_value_constraint() {
    let table = execute_optimized(
        "MATCH (a:Person)
         MATCH (a:Person)
         WHERE a.email IN ['alice@example.com', 'cara@example.com']
         RETURN a.name AS name
         ORDER BY name",
    );

    assert_eq!(
        strings(&table, "name"),
        vec!["Alice".to_owned(), "Cara".to_owned()]
    );
}

#[test]
fn seeded_composite_lookup_reapplies_value_constraint() {
    let table = execute_optimized(
        "MATCH (a:Person)
         MATCH (a:Person)
         WHERE a.tenant = 't1' AND a.kind = 'person'
         RETURN a.name AS name
         ORDER BY name",
    );

    assert_eq!(
        strings(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned()]
    );
}

#[test]
fn seeded_predicate_can_read_other_outer_columns() {
    let table = execute_optimized(
        "MATCH (a:Person), (b:Person)
         WHERE b.name = 'Alice'
         MATCH (a:Person)
         WHERE a.age > b.age
         RETURN a.name AS name
         ORDER BY name",
    );

    assert_eq!(
        strings(&table, "name"),
        vec!["Bob".to_owned(), "Cara".to_owned()]
    );
}

#[test]
fn seeded_edge_scan_reuses_outer_edge_and_predicates() {
    let table = execute_optimized(
        "MATCH ()-[r:KNOWS]->()
         MATCH ()-[r:KNOWS]->()
         WHERE r.score >= 2
         RETURN r.score AS score
         ORDER BY score",
    );

    assert_eq!(ints(&table, "score"), vec![2]);
}

#[test]
fn null_seed_keeps_correlated_subquery_empty() {
    let table = execute_optimized(
        "MATCH (a:Person)
         OPTIONAL MATCH (a)-[:KNOWS]->(m:Sensor)
         RETURN a.name AS name, EXISTS { MATCH (m)-[:KNOWS]->() } AS has_outgoing
         ORDER BY name",
    );

    assert_eq!(
        strings(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(bools(&table, "has_outgoing"), vec![false, false, false]);
}

fn graph_with_people(graph_id: u64, ages: &[i64], groups: &[&str]) -> SharedGraph {
    let graph = SharedGraph::new(selene_core::GraphId::new(graph_id));
    let person = db_string("Person");
    let id = db_string("id");
    let age = db_string("age");
    let group = db_string("grp");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for (index, (age_value, group_value)) in ages.iter().zip(groups.iter()).enumerate() {
            mutator
                .create_node(
                    selene_core::LabelSet::single(person.clone()),
                    props([
                        (id.clone(), Value::Int(index as i64)),
                        (age.clone(), Value::Int(*age_value)),
                        (group.clone(), Value::String(db_string(group_value))),
                    ]),
                )
                .expect("person inserts");
        }
        txn.commit().expect("fixture commits");
    }
    graph
        .create_property_index(person.clone(), age, TypedIndexKind::I64)
        .expect("age index builds");
    graph
        .create_property_index(person, group, TypedIndexKind::String)
        .expect("group index builds");
    graph
}
