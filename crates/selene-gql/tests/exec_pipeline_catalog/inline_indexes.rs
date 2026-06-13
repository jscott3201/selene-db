//! Inline `INDEXED` catalog coverage.

use selene_core::{Change, SchemaChange, Value};
use selene_gql::GqlStatus;
use selene_graph::TypedIndexKind;

use super::{db_string, empty_closed_graph, person_graph, planned, run_write};

#[test]
fn create_node_type_indexed_property_creates_property_index() {
    let graph = empty_closed_graph(3717);
    let plan = planned("CREATE NODE TYPE :Sensor (serial :: STRING INDEXED)");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    let outcome = outcome.expect("commit succeeds");

    let sensor = db_string("Sensor");
    let serial = db_string("serial");
    assert!(graph.read().property_index_for(&sensor, &serial).is_some());
    assert_eq!(graph.read().property_index_count(), 1);
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::SchemaChanged {
                change: SchemaChange::NodeTypeAddedV2 { label, .. },
                ..
            },
            Change::SchemaChanged {
                change: SchemaChange::PropertyIndexCreatedNamed {
                    label: index_label,
                    property,
                    name: None,
                    ..
                },
                ..
            }
        ] if label.as_str() == "Sensor"
            && index_label.as_str() == "Sensor"
            && property.as_str() == "serial"
    ));
}

#[test]
fn create_node_type_unindexed_property_does_not_create_property_index() {
    let graph = empty_closed_graph(3718);
    let plan = planned("CREATE NODE TYPE :Sensor (serial :: STRING)");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    assert_eq!(graph.read().property_index_count(), 0);
}

#[test]
fn create_node_type_bool_indexed_property_creates_property_index() {
    let graph = empty_closed_graph(3719);
    let plan = planned("CREATE NODE TYPE :Sensor (active :: BOOLEAN INDEXED)");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let sensor = db_string("Sensor");
    let active = db_string("active");
    assert_eq!(
        graph
            .read()
            .property_index_for(&sensor, &active)
            .expect("bool index exists")
            .kind(),
        TypedIndexKind::Bool
    );
}

#[test]
fn create_node_type_uint_indexed_property_creates_property_index() {
    let graph = empty_closed_graph(3720);
    let plan = planned("CREATE NODE TYPE :Sensor (reading_count :: UINT64 INDEXED)");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let sensor = db_string("Sensor");
    let reading_count = db_string("reading_count");
    assert_eq!(
        graph
            .read()
            .property_index_for(&sensor, &reading_count)
            .expect("uint index exists")
            .kind(),
        TypedIndexKind::U64
    );
}

#[test]
fn create_node_type_exact_numeric_indexed_properties_create_property_indexes() {
    let graph = empty_closed_graph(3725);
    let plan = planned(
        "CREATE NODE TYPE :Metric \
         (\"signed\" :: INT128 INDEXED, \"unsigned\" :: UINT128 INDEXED, amount :: DECIMAL INDEXED)",
    );

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    let metric = db_string("Metric");
    for (property, expected) in [
        ("amount", TypedIndexKind::Decimal),
        ("signed", TypedIndexKind::I128),
        ("unsigned", TypedIndexKind::U128),
    ] {
        assert_eq!(
            graph
                .read()
                .property_index_for(&metric, &db_string(property))
                .unwrap_or_else(|| panic!("{property} index exists"))
                .kind(),
            expected,
            "{property} index kind"
        );
    }
}

#[test]
fn create_node_type_float32_indexed_property_creates_property_index() {
    let graph = empty_closed_graph(3726);
    let plan = planned("CREATE NODE TYPE :Metric (score :: FLOAT32 INDEXED)");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog executes");
    outcome.expect("commit succeeds");

    assert_eq!(
        graph
            .read()
            .property_index_for(&db_string("Metric"), &db_string("score"))
            .expect("float32 index exists")
            .kind(),
        TypedIndexKind::F32
    );
}

#[test]
fn create_node_type_float_indexed_reports_feature_not_supported() {
    let graph = empty_closed_graph(3722);
    let plan = planned("CREATE NODE TYPE :Metric (score :: FLOAT INDEXED)");

    let err = run_write(&graph, &plan).expect_err("FLOAT inline index unsupported");

    assert_eq!(err.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
}

#[test]
fn create_edge_type_indexed_property_reports_feature_not_supported() {
    let graph = person_graph(3723);
    let plan =
        planned("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person, since :: STRING INDEXED)");

    let err = run_write(&graph, &plan).expect_err("edge inline index unsupported");

    assert_eq!(err.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
}

#[test]
fn create_node_type_duplicate_index_names_report_duplicate_object() {
    let graph = empty_closed_graph(3720);
    let plan = planned(
        "CREATE NODE TYPE :Sensor \
         (serial :: STRING INDEXED AS sensor_lookup, code :: STRING INDEXED AS sensor_lookup)",
    );

    let err = run_write(&graph, &plan).expect_err("duplicate index name rejected");

    assert_eq!(err.gqlstatus(), GqlStatus::DUPLICATE_OBJECT);
}

#[test]
fn show_indexes_renders_auto_and_explicit_inline_index_names() {
    let graph = empty_closed_graph(3721);
    let create = planned(
        "CREATE NODE TYPE :Sensor \
         (serial :: STRING INDEXED, code :: STRING INDEXED AS sensor_code_lookup)",
    );
    run_write(&graph, &create)
        .expect("catalog executes")
        .1
        .expect("commit succeeds");

    let (table, outcome) = run_write(&graph, &planned("SHOW INDEXES")).expect("show executes");
    outcome.expect("show commit succeeds");
    let rows = table.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].values(),
        &[
            Value::String(db_string("sensor_code_lookup")),
            Value::String(db_string("Sensor")),
            Value::String(db_string("code")),
            Value::String(db_string("string")),
        ]
    );
    assert_eq!(
        rows[1].values(),
        &[
            Value::String(db_string("idx:6:Sensor:6:serial")),
            Value::String(db_string("Sensor")),
            Value::String(db_string("serial")),
            Value::String(db_string("string")),
        ]
    );
}

#[test]
fn autogenerated_index_names_are_collision_safe() {
    let graph = empty_closed_graph(3724);
    run_write(
        &graph,
        &planned("CREATE NODE TYPE :A_B (c :: STRING INDEXED)"),
    )
    .expect("first catalog op executes")
    .1
    .expect("first commit succeeds");
    run_write(
        &graph,
        &planned("CREATE NODE TYPE :A (B_c :: STRING INDEXED)"),
    )
    .expect("second catalog op executes")
    .1
    .expect("second commit succeeds");

    let (table, outcome) = run_write(&graph, &planned("SHOW INDEXES")).expect("show executes");
    outcome.expect("show commit succeeds");
    let names = table
        .rows()
        .iter()
        .map(|row| match &row.values()[0] {
            Value::String(value) => value.as_str(),
            other => panic!("expected index name string, got {other:?}"),
        })
        .collect::<Vec<_>>();

    assert!(names.contains(&"idx:1:A:3:B_c"));
    assert!(names.contains(&"idx:3:A_B:1:c"));
}
