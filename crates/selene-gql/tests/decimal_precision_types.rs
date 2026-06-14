//! Conformance coverage for user-specified DECIMAL precision/scale type names.

use rust_decimal::Decimal;
use selene_core::{
    DbString, DecimalType, GraphId, PropertyValueType, Value, feature_register::FeatureId,
};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, ParserError, Session, StatementOutput,
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};
use selene_graph::{GraphTypeDef, PropertyElementType, RecordFieldType, SharedGraph};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn dec(value: &str) -> Value {
    Value::Decimal(value.parse::<Decimal>().expect("decimal parses"))
}

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_730));
    let mut session = Session::new(&graph);
    first_value_in(&mut session, source)
}

fn first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(13_731));
    let mut session = Session::new(&graph);
    let err = session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("query should fail");
    err.gqlstatus().as_str().to_owned()
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

fn bind_and_eval(value: Value, source: &str) -> Result<Value, ExecutorError> {
    let graph = SharedGraph::new(GraphId::new(13_732));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p"), value);
    let output = session.execute_source(source, &EmptyProcedureRegistry)?;
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    Ok(table.rows()[0].values()[0].clone())
}

fn assert_syntax_error(source: &str) {
    let err = parse(source).expect_err(source);
    assert!(
        matches!(err, ParserError::SyntaxError { .. }),
        "expected syntax error for `{source}`, got {err:?}"
    );
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("decimal.precision.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

#[test]
fn decimal_precision_formats_to_canonical_type_name() {
    for (source, expected) in [
        (
            "RETURN n IS TYPED DECIMAL(5)",
            "RETURN n IS TYPED DECIMAL(5)",
        ),
        (
            "RETURN n IS TYPED DECIMAL /* c */ (5)",
            "RETURN n IS TYPED DECIMAL(5)",
        ),
        (
            "RETURN n IS TYPED DEC(5, 2)",
            "RETURN n IS TYPED DECIMAL(5, 2)",
        ),
        (
            "RETURN n IS TYPED DEC /* c */ (5, 2)",
            "RETURN n IS TYPED DECIMAL(5, 2)",
        ),
        (
            "RETURN n IS TYPED DECIMAL(2_9, 2_8)",
            "RETURN n IS TYPED DECIMAL(29, 28)",
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
fn decimal_precision_keywords_require_boundaries() {
    for source in [
        "RETURN n IS TYPED DECIMALx",
        "RETURN n IS TYPED DECx",
        "RETURN n IS TYPED DECIMALx(5)",
        "RETURN n IS TYPED DECx(5, 2)",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn decimal_precision_records_gv17() {
    let observed = feature_walk(&parse("RETURN n IS TYPED DECIMAL(5, 2)").unwrap())
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(observed.contains(&FeatureId::GV17), "{observed:?}");
}

#[test]
fn decimal_precision_matches_exact_value_envelope() {
    assert_eq!(
        first_value("RETURN CAST('123.45' AS DECIMAL) IS TYPED DECIMAL(5, 2) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN CAST('123.456' AS DECIMAL) IS TYPED DECIMAL(5, 2) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN CAST('123.45' AS DECIMAL) IS TYPED DECIMAL(4, 2) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN CAST('123' AS DECIMAL) IS TYPED DECIMAL(3) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN CAST('1.2' AS DECIMAL) IS TYPED DECIMAL(3) AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn decimal_precision_casts_round_to_target_scale_then_check_precision() {
    assert_eq!(
        first_value("RETURN CAST(CAST('1.235' AS DECIMAL) AS DECIMAL(3, 2)) AS v"),
        dec("1.24")
    );
    assert_eq!(
        first_value("RETURN CAST(123 AS DECIMAL(3)) AS v"),
        dec("123")
    );
    assert_eq!(
        first_status("RETURN CAST(1234 AS DECIMAL(3)) AS v"),
        "22003"
    );
    assert_eq!(
        first_status("RETURN CAST(CAST('9.995' AS DECIMAL) AS DECIMAL(3, 2)) AS v"),
        "22003"
    );
}

#[test]
fn decimal_precision_typed_parameters_use_same_envelope() {
    assert_eq!(
        bind_and_eval(dec("1.20"), "RETURN $p :: DECIMAL(3, 2) AS v").unwrap(),
        dec("1.20")
    );
    let err = bind_and_eval(dec("10.00"), "RETURN $p :: DECIMAL(3, 2) AS v")
        .expect_err("parameter value exceeds DECIMAL(3,2)");
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}

#[test]
fn decimal_precision_persists_catalog_descriptors_and_show_rendering() {
    let graph = empty_closed_graph(13_733);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Metric (\
                amount :: DECIMAL(5, 2), \
                whole :: DEC(3), \
                history :: LIST<DECIMAL(4, 1)>, \
                payload :: RECORD { score :: DECIMAL(6, 3) }\
            )",
            &EmptyProcedureRegistry,
        )
        .expect("catalog DDL executes");

    let graph_type = graph.graph_type().expect("graph type is bound");
    let properties = &graph_type.node_types[0].properties;
    assert_eq!(properties[0].value_type, PropertyValueType::Decimal);
    assert_eq!(properties[0].decimal_type, DecimalType::new(5, 2));
    assert_eq!(properties[1].value_type, PropertyValueType::Decimal);
    assert_eq!(properties[1].decimal_type, DecimalType::new(3, 0));
    assert_eq!(
        properties[2].list_element_type,
        Some(PropertyElementType::Decimal(
            DecimalType::new(4, 1).expect("valid decimal type")
        ))
    );
    let record_fields = properties[3]
        .record_field_types
        .as_ref()
        .expect("record field descriptor");
    assert_eq!(
        record_fields.0[0].field_type,
        RecordFieldType::Decimal(DecimalType::new(6, 3).expect("valid decimal type"))
    );

    let show = session
        .execute_source("SHOW NODE TYPES", &EmptyProcedureRegistry)
        .expect("SHOW succeeds");
    let StatementOutput::Rows(table) = show else {
        panic!("SHOW returns rows");
    };
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Metric (amount :: DECIMAL(5, 2), whole :: DECIMAL(3), history :: LIST<DECIMAL(4, 1)>, payload :: RECORD { score :: DECIMAL(6, 3) })"
        ))
    );
}

#[test]
fn decimal_precision_store_assignment_rounds_and_rejects_overflow() {
    let graph = empty_closed_graph(13_734);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Metric (amount :: DECIMAL(5, 2))",
            &EmptyProcedureRegistry,
        )
        .expect("catalog DDL executes");
    session
        .execute_source(
            "INSERT (:Metric { amount: CAST('123.45' AS DECIMAL) }) FINISH",
            &EmptyProcedureRegistry,
        )
        .expect("valid decimal insert fits descriptor");
    assert_eq!(
        first_value_in(
            &mut session,
            "MATCH (m:Metric) RETURN m.amount AS amount ORDER BY amount"
        ),
        dec("123.45")
    );

    session
        .execute_source(
            "INSERT (:Metric { amount: CAST('123.456' AS DECIMAL) }) FINISH",
            &EmptyProcedureRegistry,
        )
        .expect("store assignment rounds fractional scale when representable");
    assert_eq!(
        first_value_in(
            &mut session,
            "MATCH (m:Metric) RETURN m.amount AS amount ORDER BY amount DESC"
        ),
        dec("123.46")
    );

    session
        .execute_source(
            "INSERT (:Metric { amount: 123 }) FINISH",
            &EmptyProcedureRegistry,
        )
        .expect("integer source is store-assignable to DECIMAL");

    let err = session
        .execute_source(
            "INSERT (:Metric { amount: CAST('1234.56' AS DECIMAL) }) FINISH",
            &EmptyProcedureRegistry,
        )
        .expect_err("leading digit loss is not representable");
    assert_eq!(err.gqlstatus().as_str(), "22003");
}

#[test]
fn decimal_precision_catalog_rejects_nested_values_outside_descriptor() {
    let graph = empty_closed_graph(13_736);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Metric (\
                history :: LIST<DECIMAL(3, 2)>, \
                payload :: RECORD { amount :: DECIMAL(3, 2) }\
            )",
            &EmptyProcedureRegistry,
        )
        .expect("catalog DDL executes");
    session
        .execute_source(
            "INSERT (:Metric { \
                history: [CAST('1.20' AS DECIMAL)], \
                payload: RECORD{amount: CAST('1.20' AS DECIMAL)}\
            }) FINISH",
            &EmptyProcedureRegistry,
        )
        .expect("nested decimal values fit descriptors");

    for source in [
        "INSERT (:Metric { \
            history: [CAST('10.00' AS DECIMAL)], \
            payload: RECORD{amount: CAST('1.20' AS DECIMAL)}\
        }) FINISH",
        "INSERT (:Metric { \
            history: [CAST('1.20' AS DECIMAL)], \
            payload: RECORD{amount: CAST('10.00' AS DECIMAL)}\
        }) FINISH",
    ] {
        session
            .execute_source(source, &EmptyProcedureRegistry)
            .expect_err("nested decimal value outside descriptor should fail");
    }
}

#[test]
fn decimal_precision_defaults_are_store_assigned() {
    let graph = empty_closed_graph(13_735);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Metric (\
                amount :: DECIMAL(5, 2) DEFAULT 123.456, \
                history :: LIST<DECIMAL(3, 2)> DEFAULT [1.234, 0.1], \
                payload :: RECORD { amount :: DECIMAL(3, 2) } \
                    DEFAULT RECORD{amount: 1.235}\
            )",
            &EmptyProcedureRegistry,
        )
        .expect("DEFAULT descriptor coercion succeeds");

    let show = session
        .execute_source("SHOW NODE TYPES", &EmptyProcedureRegistry)
        .expect("SHOW succeeds");
    let StatementOutput::Rows(table) = show else {
        panic!("SHOW returns rows");
    };
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Metric (amount :: DECIMAL(5, 2) DEFAULT '123.46', history :: LIST<DECIMAL(3, 2)> DEFAULT ['1.23', '0.1'], payload :: RECORD { amount :: DECIMAL(3, 2) } DEFAULT RECORD{amount: '1.24'})"
        ))
    );

    session
        .execute_source("INSERT (:Metric) FINISH", &EmptyProcedureRegistry)
        .expect("insert materializes coerced defaults");
    assert_eq!(
        first_value_in(&mut session, "MATCH (m:Metric) RETURN m.amount AS amount"),
        dec("123.46")
    );
}

#[test]
fn decimal_precision_defaults_reject_leading_digit_loss() {
    for source in [
        "CREATE NODE TYPE :Metric (amount :: DECIMAL(3, 2) DEFAULT 10.00)",
        "CREATE NODE TYPE :Metric (history :: LIST<DECIMAL(3, 2)> DEFAULT [1.20, 10.00])",
        "CREATE NODE TYPE :Metric (payload :: RECORD { amount :: DECIMAL(3, 2) } DEFAULT RECORD{amount: 10.00})",
    ] {
        let graph = empty_closed_graph(13_736);
        let mut session = Session::new(&graph);
        let err = session
            .execute_source(source, &EmptyProcedureRegistry)
            .expect_err("leading digit loss is not representable");
        assert_eq!(err.gqlstatus().as_str(), "22003");
    }
}

#[test]
fn invalid_decimal_precision_reports_honest_parse_errors() {
    for source in [
        "RETURN n IS TYPED DECIMAL(0)",
        "RETURN n IS TYPED DECIMAL(5, 6)",
        "RETURN n IS TYPED DECIMAL(30)",
        "RETURN n IS TYPED DECIMAL(29, 29)",
        "RETURN n IS TYPED DECIMAL(1__2, 0)",
        "RETURN n IS TYPED DECIMAL(12, 1__0)",
    ] {
        assert_syntax_error(source);
    }
}
