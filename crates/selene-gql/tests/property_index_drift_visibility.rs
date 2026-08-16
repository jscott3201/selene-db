//! A demoted property index must be findable from GQL (#1102).
//!
//! Since #1099 a single live row an index cannot key makes that index decline
//! every probe, so queries against it silently run as scans. Correct — the same
//! rows come back — but until `selene.property_index_stats` there was no
//! engine-visible way to learn it had happened: the index stays registered,
//! `SHOW INDEXES` still lists it unchanged, and `selene.verify` deliberately
//! audits through the drift-ignoring probe and so reports no issues.
//!
//! These tests pin the operator's actual question — *why did this query get
//! slow?* — end to end, and pin the two surfaces that were previously the only
//! places to look so their silence stays deliberate rather than accidental.

use selene_core::{DbString, GraphId, Value, db_string as core_db_string};
use selene_gql::{BindingTable, BuiltinProcedureRegistry, Session, StatementOutput};
use selene_graph::{SharedGraph, TypedIndexKind};

fn db_string(value: &str) -> DbString {
    core_db_string(value).expect("test string fits DB string cap")
}

fn rows(session: &mut Session<'_>, source: &str) -> BindingTable {
    let registry = BuiltinProcedureRegistry::new();
    match session
        .execute_source(source, &registry)
        .expect("statement executes")
    {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows from {source}, got {other:?}"),
    }
}

fn exec(session: &mut Session<'_>, source: &str) {
    let registry = BuiltinProcedureRegistry::new();
    session
        .execute_source(source, &registry)
        .expect("statement executes");
}

fn column(table: &BindingTable, name: &str) -> Vec<Value> {
    let index = table
        .column_index(db_string(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| row.values()[index].clone())
        .collect()
}

fn strings(table: &BindingTable, name: &str) -> Vec<String> {
    column(table, name)
        .into_iter()
        .map(|value| match value {
            Value::String(value) => value.as_str().to_owned(),
            other => panic!("expected string in {name}, got {other:?}"),
        })
        .collect()
}

/// Index a clean int column, then write a GQL-equal float. That ordering is how
/// a real deployment arrives here: creation is strict and would have rejected
/// the mismatched value outright.
fn drifted_graph(id: u64) -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(id));
    {
        let mut session = Session::new(&graph);
        exec(&mut session, "INSERT (:Reading { level: 3 })");
    }
    graph
        .create_property_index(
            db_string("Reading"),
            db_string("level"),
            TypedIndexKind::I64,
        )
        .expect("the index builds over the clean column");
    {
        let mut session = Session::new(&graph);
        exec(&mut session, "INSERT (:Reading { level: 3.0e0 })");
    }
    graph
}

#[test]
fn a_demoted_index_is_visible_through_the_stats_procedure() {
    let graph = drifted_graph(102_101);
    let mut session = Session::new(&graph);

    let table = rows(
        &mut session,
        "CALL selene.property_index_stats() \
         YIELD name, entity, label, properties, kind, indexed_rows, drifted_rows, answers_probes \
         RETURN name, entity, label, properties, kind, indexed_rows, drifted_rows, answers_probes",
    );

    assert_eq!(table.rows().len(), 1, "one registration, one row");
    assert_eq!(strings(&table, "entity"), ["NODE"]);
    assert_eq!(strings(&table, "label"), ["Reading"]);
    assert_eq!(strings(&table, "properties"), ["level"]);
    assert_eq!(strings(&table, "kind"), ["i64"]);
    assert_eq!(column(&table, "drifted_rows"), [Value::Uint(1)]);
    assert_eq!(
        column(&table, "answers_probes"),
        [Value::Bool(false)],
        "this is the answer to 'why did my query get slow'"
    );
}

/// The name column must join with `SHOW INDEXES`, or an operator cannot connect
/// the diagnosis to the index they know about.
#[test]
fn the_reported_name_matches_show_indexes() {
    let graph = drifted_graph(102_102);
    let mut session = Session::new(&graph);

    let shown = rows(&mut session, "SHOW INDEXES");
    let shown_names = strings(&shown, "name");

    let stats = rows(
        &mut session,
        "CALL selene.property_index_stats() YIELD name RETURN name",
    );
    let stats_names = strings(&stats, "name");

    assert_eq!(stats_names.len(), 1);
    assert!(
        shown_names.contains(&stats_names[0]),
        "stats name {:?} must appear in SHOW INDEXES {:?}",
        stats_names[0],
        shown_names
    );
}

/// A healthy index reports `answers_probes = true`, so the signal discriminates
/// rather than firing on every registered index.
#[test]
fn a_healthy_index_reports_that_it_still_answers() {
    let graph = SharedGraph::new(GraphId::new(102_103));
    {
        let mut session = Session::new(&graph);
        exec(&mut session, "INSERT (:Reading { level: 3 })");
    }
    graph
        .create_property_index(
            db_string("Reading"),
            db_string("level"),
            TypedIndexKind::I64,
        )
        .unwrap();
    {
        let mut session = Session::new(&graph);
        exec(&mut session, "INSERT (:Reading { level: 4 })");
    }

    let mut session = Session::new(&graph);
    let table = rows(
        &mut session,
        "CALL selene.property_index_stats() YIELD drifted_rows, answers_probes \
         RETURN drifted_rows, answers_probes",
    );
    assert_eq!(column(&table, "drifted_rows"), [Value::Uint(0)]);
    assert_eq!(column(&table, "answers_probes"), [Value::Bool(true)]);
}

/// `SHOW INDEXES` still shows a demoted index exactly as before.
///
/// This is deliberate, not an oversight: the registration really is intact, and
/// a catalog listing is not a health report. Pinning it means a future change
/// to that surface is a decision rather than an accident.
#[test]
fn show_indexes_still_lists_a_demoted_index_unchanged() {
    let healthy = SharedGraph::new(GraphId::new(102_104));
    {
        let mut session = Session::new(&healthy);
        exec(&mut session, "INSERT (:Reading { level: 3 })");
    }
    healthy
        .create_property_index(
            db_string("Reading"),
            db_string("level"),
            TypedIndexKind::I64,
        )
        .unwrap();

    let healthy_rows = {
        let mut session = Session::new(&healthy);
        let table = rows(&mut session, "SHOW INDEXES");
        strings(&table, "name")
    };

    let drifted = drifted_graph(102_105);
    let drifted_rows = {
        let mut session = Session::new(&drifted);
        let table = rows(&mut session, "SHOW INDEXES");
        strings(&table, "name")
    };

    assert_eq!(
        healthy_rows, drifted_rows,
        "SHOW INDEXES is a catalog listing, so drift must not change it"
    );
}

/// `selene.verify` still reports no issue for a demoted index.
///
/// Also deliberate: verify is a corruption audit, and an index declining probes
/// is behaving as designed rather than being corrupt. The drift count is now
/// carried in the check's detail so the audit is not silent about it, but the
/// status stays `ok`.
#[test]
fn verify_reports_ok_but_names_the_drift_in_its_detail() {
    let graph = drifted_graph(102_106);
    let mut session = Session::new(&graph);

    let table = rows(
        &mut session,
        "CALL selene.verify() YIELD check, status, detail RETURN check, status, detail",
    );
    let checks = strings(&table, "check");
    let statuses = strings(&table, "status");
    let details = strings(&table, "detail");

    let position = checks
        .iter()
        .position(|check| check == "property_index_coverage")
        .expect("the coverage check runs");
    assert_eq!(
        statuses[position], "ok",
        "a demoted index is not corrupt, so the audit status stays ok"
    );
    assert!(
        details[position].contains("drifted rows=1"),
        "the audit must not be silent about drift: {:?}",
        details[position]
    );
}
