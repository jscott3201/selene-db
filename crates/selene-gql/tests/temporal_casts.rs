//! Temporal CAST conformance cases.

use selene_core::{GraphId, Value, intern};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::SharedGraph;

fn cast_bound_to_string(value: Value) -> String {
    let graph = SharedGraph::new(GraphId::new(13_810));
    let mut session = Session::new(&graph);
    session.bind_parameter(intern("p").expect("intern param"), value);
    let output = session
        .execute_source("RETURN CAST($p AS STRING) AS v", &EmptyProcedureRegistry)
        .expect("temporal string cast succeeds");
    let StatementOutput::Rows(table) = output else {
        panic!("temporal string cast produced non-row output");
    };
    let Value::String(value) = table.rows()[0].values()[0].clone() else {
        panic!("temporal string cast did not return STRING");
    };
    value.as_str().to_owned()
}

#[test]
fn cast_temporal_instants_to_strings() {
    assert_eq!(
        cast_bound_to_string(Value::Date("2026-05-07".parse().unwrap())),
        "2026-05-07"
    );
    assert_eq!(
        cast_bound_to_string(Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap())),
        "2026-05-07T12:34:56"
    );
    assert_eq!(
        cast_bound_to_string(Value::LocalTime("12:34:56".parse().unwrap())),
        "12:34:56"
    );
}

#[test]
fn cast_zoned_temporal_instants_omit_zone_annotation() {
    let zoned = "2026-05-07T12:34:56-04:00[America/New_York]";
    assert_eq!(
        cast_bound_to_string(Value::ZonedDateTime(Box::new(zoned.parse().unwrap()))),
        "2026-05-07T12:34:56-04"
    );
    assert_eq!(
        cast_bound_to_string(Value::ZonedTime(Box::new(zoned.parse().unwrap()))),
        "12:34:56-04"
    );
}

#[test]
fn cast_durations_to_iso_strings() {
    assert_eq!(
        cast_bound_to_string(Value::Duration(Box::new("P2M".parse().unwrap()))),
        "P2M"
    );
    assert_eq!(
        cast_bound_to_string(Value::Duration(Box::new("PT1H2S".parse().unwrap()))),
        "PT1H2S"
    );
}
