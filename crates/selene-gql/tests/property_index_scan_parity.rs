//! Indexed reads must return exactly the rows an unindexed read returns.
//!
//! GQL equality is cross-variant: `3` equals `3.0`. A typed property index keys
//! one variant, so on an open graph a column can hold rows the index cannot
//! key. Those rows are absent from the index but present to a scan, and the
//! optimizer picks between the two on the literal's type — so the same
//! predicate written two ways could disagree on identical data.
//!
//! An index is supposed to make a query faster, never different. These tests
//! pin that for variant drift, which is what the surrounding change addresses.
//!
//! One index-versus-scan divergence is out of scope and still open: `-0.0` and
//! `0.0` are equal under GQL comparison but key distinctly in a float index, so
//! an indexed equality read can still disagree with a scan without any row
//! being unkeyable. That is a separate defect in float key construction, not a
//! completeness problem, and the tally cannot see it.

use selene_core::{DbString, GraphId, Value, db_string as core_db_string};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::{SharedGraph, TypedIndexKind};

fn db_string(value: &str) -> DbString {
    core_db_string(value).expect("test string fits DB string cap")
}

fn run(session: &mut Session<'_>, source: &str) -> selene_gql::BindingTable {
    match session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("statement executes")
    {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows from {source}, got {other:?}"),
    }
}

fn exec(session: &mut Session<'_>, source: &str) {
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("statement executes");
}

/// Row count for a `RETURN n` style query.
fn count(session: &mut Session<'_>, source: &str) -> usize {
    run(session, source).rows().len()
}

/// An open graph holding one Int and one Float row that are GQL-equal, with an
/// I64 index registered before the Float row lands.
///
/// The index is created while only the Int row exists, because index creation
/// is strict and would reject the mismatched value outright. That ordering is
/// exactly how a real deployment arrives here: index a clean column, then write
/// a value of another numeric type.
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
        .expect("index builds over the clean column");
    {
        let mut session = Session::new(&graph);
        exec(&mut session, "INSERT (:Reading { level: 3.0e0 })");
    }
    graph
}

#[test]
fn integer_and_float_spellings_of_one_predicate_agree() {
    let graph = drifted_graph(90_101);
    let mut session = Session::new(&graph);

    let via_int_literal = count(&mut session, "MATCH (n:Reading) WHERE n.level = 3 RETURN n");
    let via_float_literal = count(
        &mut session,
        "MATCH (n:Reading) WHERE n.level = 3.0e0 RETURN n",
    );

    assert_eq!(
        via_int_literal, via_float_literal,
        "the same predicate written two ways must return the same rows; the \
         optimizer picks an index scan for one spelling and a linear scan for \
         the other, and an index that omits rows makes them disagree"
    );
    assert_eq!(
        via_int_literal, 2,
        "both stored values are equal to 3 under GQL cross-variant equality"
    );
}

#[test]
fn indexed_label_matches_unindexed_label_on_identical_data() {
    let indexed = drifted_graph(90_102);

    let unindexed = SharedGraph::new(GraphId::new(90_103));
    {
        let mut session = Session::new(&unindexed);
        exec(&mut session, "INSERT (:Reading { level: 3 })");
        exec(&mut session, "INSERT (:Reading { level: 3.0e0 })");
    }

    let query = "MATCH (n:Reading) WHERE n.level = 3 RETURN n";
    let mut indexed_session = Session::new(&indexed);
    let mut unindexed_session = Session::new(&unindexed);

    assert_eq!(
        count(&mut indexed_session, query),
        count(&mut unindexed_session, query),
        "registering an index must not change which rows a query returns"
    );
}

#[test]
fn range_predicates_agree_across_the_same_drift() {
    let graph = drifted_graph(90_104);
    let mut session = Session::new(&graph);

    assert_eq!(
        count(&mut session, "MATCH (n:Reading) WHERE n.level > 2 RETURN n"),
        2,
        "a range predicate must also see the row the index cannot key"
    );
}

#[test]
fn a_clean_index_still_answers_and_stays_exact() {
    let graph = SharedGraph::new(GraphId::new(90_105));
    {
        let mut session = Session::new(&graph);
        exec(&mut session, "INSERT (:Reading { level: 3 })");
        exec(&mut session, "INSERT (:Reading { level: 4 })");
    }
    graph
        .create_property_index(
            db_string("Reading"),
            db_string("level"),
            TypedIndexKind::I64,
        )
        .expect("index builds");

    let snapshot = graph.read();
    let reading = db_string("Reading");
    let level = db_string("level");
    assert!(
        snapshot
            .nodes_with_property_eq(&reading, &level, &Value::Int(3))
            .is_some(),
        "an index covering every row must keep answering probes"
    );

    let mut session = Session::new(&graph);
    assert_eq!(
        count(&mut session, "MATCH (n:Reading) WHERE n.level = 3 RETURN n"),
        1
    );
}

#[test]
fn repairing_the_drifted_row_re_enables_the_index() {
    let graph = drifted_graph(90_106);
    let reading = db_string("Reading");
    let level = db_string("level");

    assert!(
        graph
            .read()
            .nodes_with_property_eq(&reading, &level, &Value::Int(3))
            .is_none(),
        "while a row is unkeyable the index declines so callers scan"
    );

    // Both stored values are GQL-equal, so the unkeyable one cannot be singled
    // out by predicate. Clear the label and rebuild it cleanly instead, which
    // is what an operator repairing the column would do.
    {
        let mut session = Session::new(&graph);
        exec(&mut session, "MATCH (n:Reading) DETACH DELETE n");
    }
    {
        let mut session = Session::new(&graph);
        exec(&mut session, "INSERT (:Reading { level: 7 })");
    }
    assert!(
        graph
            .read()
            .nodes_with_property_eq(&reading, &level, &Value::Int(7))
            .is_some(),
        "once every unkeyable row is gone the index answers again"
    );
}
