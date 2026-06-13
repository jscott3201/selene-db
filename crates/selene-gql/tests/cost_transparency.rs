//! OPT-5 semantic-transparency harness: a read corpus must return byte-identical
//! result rows whether the optimizer's index selection (and its cost model) is
//! ON or OFF, over both a uniform and a heavily-skewed graph. The cost model can
//! only change WHICH correct access path runs — never WHICH rows are produced.

use selene_core::{GraphId, db_string};
use selene_gql::{BindingTable, EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::{SharedGraph, TypedIndexKind};
use smallvec::smallvec;

fn run(session: &mut Session<'_>, source: &str) -> BindingTable {
    match session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("statement executes")
    {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("write returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn exec_write(session: &mut Session<'_>, source: &str) {
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("ddl/write executes");
}

/// A multiset of result rows rendered as sortable strings, so two tables compare
/// equal iff they hold the same rows (order-independent for plain MATCH … RETURN
/// without ORDER BY).
fn row_multiset(table: &BindingTable) -> Vec<String> {
    let mut rows: Vec<String> = table
        .rows()
        .iter()
        .map(|binding| format!("{:?}", binding.values()))
        .collect();
    rows.sort();
    rows
}

/// Build a graph, register indexes, and seed data via GQL writes on a throwaway
/// session, returning the populated shared graph.
fn build_graph(id: u64, skewed: bool) -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(id));
    let person = db_string("Person").unwrap();
    let age = db_string("age").unwrap();
    let city = db_string("city").unwrap();

    // Open-graph indexes are created via the graph API (CREATE INDEX DDL needs a
    // bound graph type). Single-property (age, city) + composite (city, age).
    graph
        .create_property_index(person.clone(), age.clone(), TypedIndexKind::I64)
        .unwrap();
    graph
        .create_property_index(person.clone(), city.clone(), TypedIndexKind::String)
        .unwrap();
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_composite_property_index_named(
                person,
                smallvec![city, age],
                smallvec![TypedIndexKind::String, TypedIndexKind::I64],
                Some(db_string("person_city_age").unwrap()),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    {
        let mut session = Session::new(&graph);
        // Seed 60 Person rows. Uniform: ages spread 0..30, cities A..F.
        // Skewed: most rows in city 'A' age 0 (one giant bucket), a few unique.
        for i in 0..60i64 {
            let (age, city) = if skewed {
                if i < 50 { (0i64, "A") } else { (i, "B") }
            } else {
                (i % 30, ["A", "B", "C", "D", "E", "F"][(i % 6) as usize])
            };
            exec_write(
                &mut session,
                &format!("INSERT (n:Person {{ id: {i}, age: {age}, city: '{city}' }})"),
            );
        }
        // A handful of Company nodes for label-only / disjunctive cases.
        for i in 0..5i64 {
            exec_write(&mut session, &format!("INSERT (c:Company {{ id: {i} }})"));
        }
    }
    graph
}

/// The read corpus exercised under every (graph, index-selection) combination.
const CORPUS: &[&str] = &[
    // Label-only.
    "MATCH (n:Person) RETURN n.id AS id",
    "MATCH (c:Company) RETURN c.id AS id",
    // Literal equality.
    "MATCH (n:Person) FILTER n.age = 0 RETURN n.id AS id",
    "MATCH (n:Person) FILTER n.city = 'A' RETURN n.id AS id",
    // Single-ended + double-ended range.
    "MATCH (n:Person) FILTER n.age > 10 RETURN n.id AS id",
    "MATCH (n:Person) FILTER n.age >= 5 AND n.age < 15 RETURN n.id AS id",
    // Composite (full key) + partial-subset.
    "MATCH (n:Person) FILTER n.city = 'A' AND n.age = 0 RETURN n.id AS id",
    "MATCH (n:Person) FILTER n.city = 'B' AND n.age = 50 AND n.id = 50 RETURN n.id AS id",
    // Small + larger IN-list.
    "MATCH (n:Person) FILTER n.age IN [0, 1, 2] RETURN n.id AS id",
    "MATCH (n:Person) FILTER n.city IN ['A', 'B', 'C'] RETURN n.id AS id",
    // Disjunctive labels.
    "MATCH (n:Person|Company) RETURN n.id AS id",
    // No matching rows.
    "MATCH (n:Person) FILTER n.age = 9999 RETURN n.id AS id",
    // Combined predicates.
    "MATCH (n:Person) FILTER n.age > 0 AND n.city = 'A' RETURN n.id AS id",
];

fn assert_corpus_identical(graph: &SharedGraph, label: &str) {
    for source in CORPUS {
        let mut on = Session::new(graph).with_index_selection();
        let mut off = Session::new(graph).without_index_selection();
        let on_rows = row_multiset(&run(&mut on, source));
        let off_rows = row_multiset(&run(&mut off, source));
        assert_eq!(
            on_rows, off_rows,
            "[{label}] index-selection ON vs OFF diverged for query: {source}"
        );
    }
}

#[test]
fn uniform_graph_results_identical_on_vs_off() {
    let graph = build_graph(9_910, false);
    assert_corpus_identical(&graph, "uniform");
}

#[test]
fn skewed_graph_results_identical_on_vs_off() {
    // The skewed graph (one giant city='A'/age=0 bucket + a few unique rows) is
    // where the cost model is most likely to flip the chosen access path; the
    // results must still match the pure-Linear path exactly.
    let graph = build_graph(9_911, true);
    assert_corpus_identical(&graph, "skewed");
}

#[test]
fn ordered_results_identical_on_vs_off() {
    // ORDER BY pins row ORDER too (not just the multiset), so a divergent access
    // path would surface as a different sequence, not just a different set.
    let graph = build_graph(9_912, false);
    let queries = [
        "MATCH (n:Person) RETURN n.id AS id ORDER BY n.id",
        "MATCH (n:Person) FILTER n.age > 5 RETURN n.id AS id ORDER BY n.id",
        "MATCH (n:Person) FILTER n.city = 'A' RETURN n.id AS id ORDER BY n.age, n.id",
    ];
    for source in queries {
        let mut on = Session::new(&graph).with_index_selection();
        let mut off = Session::new(&graph).without_index_selection();
        let on_rows: Vec<String> = run(&mut on, source)
            .rows()
            .iter()
            .map(|b| format!("{:?}", b.values()))
            .collect();
        let off_rows: Vec<String> = run(&mut off, source)
            .rows()
            .iter()
            .map(|b| format!("{:?}", b.values()))
            .collect();
        assert_eq!(on_rows, off_rows, "ordered divergence for: {source}");
    }
}
