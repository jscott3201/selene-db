//! Conformance coverage for ISO specified floating-point precision type names.

use selene_core::{DbString, GraphId, PropertyValueType, Value, feature_register::FeatureId};
use selene_gql::{
    EmptyProcedureRegistry, ParserError, Session, StatementOutput,
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};
use selene_graph::{GraphTypeDef, SharedGraph};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_727));
    let mut session = Session::new(&graph);
    first_value_in(&mut session, source)
}

fn first_value_in(session: &mut Session<'_>, source: &str) -> Value {
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn bind_and_eval(value: Value, source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_728));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p"), value);
    first_value_in(&mut session, source)
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("float.precision.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

#[test]
fn specified_float_precision_formats_to_supported_normal_form() {
    for (source, expected) in [
        ("RETURN n IS TYPED FLOAT(1)", "RETURN n IS TYPED FLOAT32"),
        ("RETURN n IS TYPED FLOAT(23)", "RETURN n IS TYPED FLOAT32"),
        ("RETURN n IS TYPED FLOAT(1, 0)", "RETURN n IS TYPED FLOAT32"),
        ("RETURN n IS TYPED FLOAT(24)", "RETURN n IS TYPED FLOAT64"),
        ("RETURN n IS TYPED FLOAT(52)", "RETURN n IS TYPED FLOAT64"),
        (
            "RETURN n IS TYPED FLOAT(2_3, 7)",
            "RETURN n IS TYPED FLOAT32",
        ),
        (
            "RETURN n IS TYPED FLOAT(23, 8)",
            "RETURN n IS TYPED FLOAT64",
        ),
        (
            "RETURN n IS TYPED FLOAT(52, 10)",
            "RETURN n IS TYPED FLOAT64",
        ),
    ] {
        let parsed = parse(source).expect(source);
        let formatted = format_read_statement(&parsed).expect("read statement formats");
        assert_eq!(formatted, expected);
        let reparsed = parse(&formatted).expect("formatted source parses");
        assert!(structurally_eq(&parsed, &reparsed), "{source}");
    }
}

#[test]
fn specified_float_precision_records_gv22_and_normal_form_width() {
    for (source, expected_width_feature) in [
        ("RETURN n IS TYPED FLOAT(23, 7)", FeatureId::GV21),
        ("RETURN n IS TYPED FLOAT(52, 10)", FeatureId::GV24),
    ] {
        let observed = feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&FeatureId::GV22),
            "{source} must flag GV22; observed {observed:?}"
        );
        assert!(
            observed.contains(&expected_width_feature),
            "{source} must flag {expected_width_feature:?}; observed {observed:?}"
        );
    }
}

#[test]
fn specified_float_precision_uses_selected_runtime_width() {
    assert_eq!(
        first_value("RETURN CAST(1.5 AS FLOAT(23)) AS v"),
        Value::Float32(1.5_f32)
    );
    assert_eq!(
        first_value("RETURN CAST(1.5 AS FLOAT(24)) AS v"),
        Value::Float(1.5)
    );
    assert_eq!(
        first_value("RETURN CAST(1.5 AS FLOAT(23, 8)) AS v"),
        Value::Float(1.5)
    );
    assert_eq!(
        first_value("RETURN CAST(1.5 AS FLOAT(52, 10)) IS TYPED FLOAT64 AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        bind_and_eval(
            Value::Float32(1.25_f32),
            "RETURN $p IS TYPED FLOAT(23, 7) AS ok"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        bind_and_eval(
            Value::Float32(1.25_f32),
            "RETURN $p IS TYPED FLOAT(23, 8) AS ok"
        ),
        Value::Bool(false)
    );
}

#[test]
fn specified_float_precision_lowers_catalog_property_types() {
    let graph = empty_closed_graph(13_729);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Metric (\"small\" :: FLOAT(23, 7), wide :: FLOAT(24), scaled :: FLOAT(23, 8))",
            &EmptyProcedureRegistry,
        )
        .expect("catalog DDL executes");

    let graph_type = graph.graph_type().expect("graph type is bound");
    let properties = &graph_type.node_types[0].properties;
    assert_eq!(properties[0].value_type, PropertyValueType::Float32);
    assert_eq!(properties[1].value_type, PropertyValueType::Float);
    assert_eq!(properties[2].value_type, PropertyValueType::Float);

    let show = session
        .execute_source("SHOW NODE TYPES", &EmptyProcedureRegistry)
        .expect("SHOW succeeds");
    let StatementOutput::Rows(table) = show else {
        panic!("SHOW returns rows");
    };
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Metric (\"small\" :: FLOAT32, wide :: FLOAT, scaled :: FLOAT)"
        ))
    );
}

#[test]
fn invalid_or_unsupported_float_precision_reports_honest_parse_errors() {
    for source in [
        "RETURN n IS TYPED FLOAT(0)",
        "RETURN n IS TYPED FLOAT(7_)",
        "RETURN n IS TYPED FLOAT(1__2)",
        "RETURN n IS TYPED FLOAT(10, 1__0)",
        "RETURN n IS TYPED FLOAT(10, 11)",
    ] {
        let err = parse(source).expect_err(source);
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected syntax error for `{source}`, got {err:?}"
        );
    }

    for (source, expected_feature) in [
        ("RETURN n IS TYPED FLOAT(53)", FeatureId::GV25),
        ("RETURN n IS TYPED FLOAT(23, 11)", FeatureId::GV25),
        ("RETURN n IS TYPED FLOAT(112, 14)", FeatureId::GV25),
        ("RETURN n IS TYPED FLOAT(113)", FeatureId::GV26),
        ("RETURN n IS TYPED FLOAT(15, 15)", FeatureId::GV26),
    ] {
        let err = parse(source).expect_err(source);
        let ParserError::UnsupportedFeature { feature_id, .. } = err else {
            panic!("expected unsupported feature for `{source}`, got {err:?}");
        };
        assert_eq!(feature_id, expected_feature);
    }
}
