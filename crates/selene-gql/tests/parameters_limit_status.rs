//! LIMIT/OFFSET parameter GQLSTATUS regressions.

use selene_core::{GraphId, Value, db_string};
use selene_gql::{
    DataExceptionSubclass, EmptyProcedureRegistry, ExecutorError, GqlStatus, Session,
};
use selene_graph::SharedGraph;

fn empty_graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

#[test]
fn offset_parameter_negative_reports_negative_limit_value() {
    let graph = empty_graph(12_900);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("offset").expect("test string fits"),
        Value::Int(-1),
    );

    let err = session
        .execute_source(
            "MATCH (n:Sensor) RETURN n OFFSET $offset LIMIT 1",
            &EmptyProcedureRegistry,
        )
        .expect_err("negative offset is rejected");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::NegativeLimitValue,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::NEGATIVE_LIMIT_VALUE);
}

#[test]
fn limit_parameter_null_reports_null_value_not_allowed() {
    let graph = empty_graph(12_901);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count").expect("test string fits"), Value::Null);

    let err = session
        .execute_source(
            "MATCH (n:Sensor) RETURN n LIMIT $count",
            &EmptyProcedureRegistry,
        )
        .expect_err("null limit is rejected");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::NullValueNotAllowed,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::NULL_VALUE_NOT_ALLOWED);
}

#[test]
fn typed_limit_parameter_null_reports_null_value_not_allowed() {
    let graph = empty_graph(12_902);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count").expect("test string fits"), Value::Null);

    let err = session
        .execute_source(
            "MATCH (n:Sensor) RETURN n LIMIT $count :: INT",
            &EmptyProcedureRegistry,
        )
        .expect_err("typed NULL LIMIT parameter is rejected by limit semantics");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::NullValueNotAllowed,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::NULL_VALUE_NOT_ALLOWED);
}
