//! Temporal inline index catalog coverage.

use selene_core::Value;
use selene_graph::TypedIndexKind;

use super::{db_string, empty_closed_graph, planned, run_write};

#[test]
fn create_node_type_temporal_time_indexed_properties_create_property_indexes() {
    let graph = empty_closed_graph(3727);
    let plan = planned(
        "CREATE NODE TYPE :Event \
         (occurred_at :: ZONED DATETIME INDEXED, wall_time :: LOCAL TIME INDEXED, \
          clock_time :: ZONED TIME INDEXED, span :: DURATION (DAY TO SECOND) INDEXED, \
          billing_cycle :: DURATION (YEAR TO MONTH) INDEXED, \
          elapsed :: DURATION (DAY TO SECOND) INDEXED)",
    );

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let event = db_string("Event");
    for (property, expected) in [
        ("clock_time", TypedIndexKind::ZonedTime),
        ("billing_cycle", TypedIndexKind::Duration),
        ("elapsed", TypedIndexKind::Duration),
        ("occurred_at", TypedIndexKind::ZonedDateTime),
        ("span", TypedIndexKind::Duration),
        ("wall_time", TypedIndexKind::LocalTime),
    ] {
        assert_eq!(
            graph
                .read()
                .property_index_for(&event, &db_string(property))
                .unwrap_or_else(|| panic!("{property} index exists"))
                .kind(),
            expected,
            "{property} index kind"
        );
    }

    let (table, outcome) = run_write(&graph, &planned("SHOW INDEXES")).expect("show executes");
    outcome.expect("show commit succeeds");
    let rendered = table
        .rows()
        .iter()
        .map(|row| match &row.values()[3] {
            Value::String(kind) => kind.as_str().to_owned(),
            other => panic!("expected index kind string, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert!(rendered.contains(&"local_time".to_owned()));
    assert!(rendered.contains(&"zoned_datetime".to_owned()));
    assert!(rendered.contains(&"zoned_time".to_owned()));
    assert!(rendered.contains(&"duration".to_owned()));
}
