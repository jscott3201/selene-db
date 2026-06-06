//! Runtime conformance for NULL/NOTHING cast and typed-predicate boundaries.

use selene_core::{GraphId, Value};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::SharedGraph;

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_800));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(13_801));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

#[test]
fn cast_null_to_null_and_nothing_targets_propagates_null() {
    for source in [
        "RETURN CAST(NULL AS NULL) AS v",
        "RETURN CAST(NULL AS NOTHING) AS v",
    ] {
        assert_eq!(first_value(source), Value::Null, "{source}");
    }
}

#[test]
fn cast_non_null_to_null_and_nothing_targets_remains_unsupported() {
    for source in [
        "RETURN CAST(1 AS NULL) AS v",
        "RETURN CAST(1 AS NOTHING) AS v",
    ] {
        assert_eq!(first_status(source), "42N01", "{source}");
    }
}

#[test]
fn null_is_typed_null_but_not_nothing() {
    assert_eq!(
        first_value("RETURN NULL IS TYPED NULL AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN NULL IS TYPED NOTHING AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN NULL IS NOT TYPED NOTHING AS ok"),
        Value::Bool(true)
    );
}
