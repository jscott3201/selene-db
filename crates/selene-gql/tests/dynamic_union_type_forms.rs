//! ISO dynamic union value type coverage.

use selene_core::{GraphId, Value, db_string};
use selene_gql::{
    EmptyProcedureRegistry, GqlStatus, GqlType, IsCheckKind, PipelineStatement, Session, Statement,
    StatementOutput, ValueExpr,
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};
use selene_graph::SharedGraph;
use selene_profile::FeatureId;

#[test]
fn open_dynamic_union_type_forms_parse_to_ast() {
    for source in [
        "RETURN NULL IS TYPED ANY AS ok",
        "RETURN NULL IS TYPED ANY VALUE AS ok",
    ] {
        assert_eq!(typed_type(source), GqlType::Any, "{source}");
    }

    for source in [
        "RETURN NULL IS TYPED PROPERTY VALUE AS ok",
        "RETURN NULL IS TYPED ANY PROPERTY VALUE AS ok",
    ] {
        assert_eq!(typed_type(source), GqlType::AnyProperty, "{source}");
    }
}

#[test]
fn open_dynamic_union_type_keywords_accept_comment_boundaries() {
    for (source, expected) in [
        (
            "RETURN NULL IS TYPED ANY /* boundary */ VALUE AS ok",
            GqlType::Any,
        ),
        (
            "RETURN NULL IS TYPED PROPERTY /* boundary */ VALUE AS ok",
            GqlType::AnyProperty,
        ),
        (
            "RETURN NULL IS TYPED ANY /* boundary */ PROPERTY /* boundary */ VALUE AS ok",
            GqlType::AnyProperty,
        ),
    ] {
        assert_eq!(typed_type(source), expected, "{source}");
    }
}

#[test]
fn open_dynamic_union_type_forms_format_canonically() {
    for (source, expected) in [
        (
            "RETURN NULL IS TYPED ANY VALUE AS ok",
            "RETURN null IS TYPED ANY AS ok",
        ),
        (
            "RETURN NULL IS TYPED PROPERTY VALUE AS ok",
            "RETURN null IS TYPED ANY PROPERTY VALUE AS ok",
        ),
        (
            "RETURN NULL IS TYPED ANY PROPERTY VALUE NOT NULL AS ok",
            "RETURN null IS TYPED ANY PROPERTY VALUE NOT NULL AS ok",
        ),
    ] {
        let parsed = parse(source).unwrap_or_else(|error| panic!("{source} parses: {error:?}"));
        let formatted = format_read_statement(&parsed).expect("read-side AST formats");
        assert_eq!(formatted, expected, "{source}");
        let reparsed =
            parse(&formatted).unwrap_or_else(|error| panic!("{formatted} reparses: {error:?}"));
        assert!(
            structurally_eq(&parsed, &reparsed),
            "{source} should round-trip through {formatted}"
        );
    }
}

#[test]
fn open_dynamic_union_predicates_enforce_membership() {
    assert_eq!(
        first_value("RETURN 1 IS TYPED ANY AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN NULL IS TYPED ANY NOT NULL AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN {a: 1} IS TYPED PROPERTY VALUE AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN NULL IS TYPED ANY PROPERTY VALUE NOT NULL AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn open_dynamic_union_casts_are_identity_membership_checks() {
    assert_eq!(first_value("RETURN CAST(1 AS ANY) AS v"), Value::Int(1));
    let graph = SharedGraph::new(GraphId::new(16_608));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("xs").expect("valid parameter name"),
        Value::List(vec![
            Value::Int(1),
            Value::String(db_string("x").expect("valid string")),
            Value::Bool(true),
        ]),
    );
    let output = session
        .execute_source("RETURN CAST($xs AS LIST) AS v", &EmptyProcedureRegistry)
        .expect("mixed list parameter casts through LIST<ANY>");
    assert_eq!(
        first_output_value(output),
        Value::List(vec![
            Value::Int(1),
            Value::String(db_string("x").expect("db string")),
            Value::Bool(true),
        ])
    );
    assert_eq!(
        first_value("RETURN CAST({a: 1} AS ANY PROPERTY VALUE) AS v"),
        first_value("RETURN {a: 1} AS v")
    );
}

#[test]
fn closed_dynamic_union_type_forms_parse_to_ast() {
    assert_eq!(
        typed_type("RETURN NULL IS TYPED STRING | INTEGER AS ok"),
        GqlType::ClosedDynamicUnion(vec![GqlType::String, GqlType::Integer])
    );
    assert_eq!(
        typed_type("RETURN NULL IS TYPED ANY<STRING | INTEGER> AS ok"),
        GqlType::ClosedDynamicUnion(vec![GqlType::String, GqlType::Integer])
    );
    assert_eq!(
        typed_type("RETURN NULL IS TYPED ANY VALUE < STRING NOT NULL | INTEGER NOT NULL > AS ok"),
        GqlType::ClosedDynamicUnion(vec![
            GqlType::NotNull(Box::new(GqlType::String)),
            GqlType::NotNull(Box::new(GqlType::Integer)),
        ])
    );
    assert_eq!(
        typed_type("RETURN NULL IS TYPED LIST<STRING | INTEGER> AS ok"),
        GqlType::List(Box::new(GqlType::ClosedDynamicUnion(vec![
            GqlType::String,
            GqlType::Integer,
        ])))
    );
}

#[test]
fn closed_dynamic_union_type_forms_format_canonically() {
    for (source, expected) in [
        (
            "RETURN NULL IS TYPED ANY<STRING | INTEGER> AS ok",
            "RETURN null IS TYPED STRING | INTEGER AS ok",
        ),
        (
            "RETURN NULL IS TYPED ANY VALUE < STRING NOT NULL | INTEGER NOT NULL > AS ok",
            "RETURN null IS TYPED STRING NOT NULL | INTEGER NOT NULL AS ok",
        ),
        (
            "RETURN NULL IS TYPED LIST<STRING | INTEGER> AS ok",
            "RETURN null IS TYPED LIST<STRING | INTEGER> AS ok",
        ),
    ] {
        let parsed = parse(source).unwrap_or_else(|error| panic!("{source} parses: {error:?}"));
        let formatted = format_read_statement(&parsed).expect("read-side AST formats");
        assert_eq!(formatted, expected, "{source}");
        let reparsed =
            parse(&formatted).unwrap_or_else(|error| panic!("{formatted} reparses: {error:?}"));
        assert!(
            structurally_eq(&parsed, &reparsed),
            "{source} should round-trip through {formatted}"
        );
    }
}

#[test]
fn closed_dynamic_union_predicates_enforce_membership_and_nullability() {
    assert_eq!(
        first_value("RETURN 1 IS TYPED STRING | INTEGER AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN 'x' IS TYPED STRING | INTEGER AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN true IS TYPED STRING | INTEGER AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN NULL IS TYPED STRING | INTEGER AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN NULL IS TYPED STRING NOT NULL | INTEGER NOT NULL AS ok"),
        Value::Bool(false)
    );
    let graph = SharedGraph::new(GraphId::new(16_610));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("xs").expect("valid parameter name"),
        Value::List(vec![
            Value::Int(1),
            Value::String(db_string("x").expect("valid string")),
        ]),
    );
    let output = session
        .execute_source(
            "RETURN $xs IS TYPED LIST<STRING | INTEGER> AS ok",
            &EmptyProcedureRegistry,
        )
        .expect("mixed parameter list can be checked against nested closed union");
    assert_eq!(first_output_value(output), Value::Bool(true));
    assert_eq!(
        first_value("RETURN [true] IS TYPED LIST<STRING | INTEGER> AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn closed_dynamic_union_casts_are_identity_membership_checks() {
    assert_eq!(
        first_value("RETURN CAST(1 AS STRING | INTEGER) AS v"),
        Value::Int(1)
    );
    assert_eq!(
        status_for("RETURN CAST(true AS STRING | INTEGER) AS v"),
        GqlStatus::DATATYPE_MISMATCH
    );
    assert_eq!(
        status_for("RETURN CAST(NULL AS STRING NOT NULL | INTEGER NOT NULL) AS v"),
        GqlStatus::DATATYPE_MISMATCH
    );
}

#[test]
fn closed_dynamic_union_rejects_non_conforming_component_lists() {
    for source in [
        "RETURN NULL IS TYPED ANY<STRING> AS ok",
        "RETURN NULL IS TYPED STRING | INTEGER NOT NULL AS ok",
        "RETURN NULL IS TYPED ANY<STRING | INTEGER NOT NULL> AS ok",
    ] {
        let err = parse(source).expect_err("non-conforming closed union should reject");
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}

#[test]
fn dynamic_union_type_forms_record_iso_features() {
    let ids = |source: &str| {
        feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };

    let open = ids("RETURN NULL IS TYPED ANY AS ok");
    assert!(open.contains(&FeatureId::GV66), "observed {open:?}");

    let property = ids("RETURN NULL IS TYPED PROPERTY VALUE AS ok");
    assert!(property.contains(&FeatureId::GV68), "observed {property:?}");

    let closed = ids("RETURN NULL IS TYPED STRING | INTEGER AS ok");
    assert!(closed.contains(&FeatureId::GV67), "observed {closed:?}");

    let bare_list = ids("RETURN NULL IS TYPED LIST AS ok");
    assert!(
        bare_list.contains(&FeatureId::GV50) && bare_list.contains(&FeatureId::GV66),
        "bare LIST is LIST<ANY> and must flag GV50 + GV66; observed {bare_list:?}"
    );
}

fn typed_type(source: &str) -> GqlType {
    let statement =
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    let Statement::Query(pipeline) = statement else {
        panic!("{source} should parse as a query");
    };
    let PipelineStatement::Return(return_clause) = &pipeline.statements[0] else {
        panic!("{source} should parse as RETURN");
    };
    let ValueExpr::IsCheck {
        kind: IsCheckKind::Typed(ty),
        ..
    } = &return_clause.items[0].expr
    else {
        panic!("{source} should parse as IS TYPED");
    };
    ty.clone()
}

fn status_for(source: &str) -> GqlStatus {
    let graph = SharedGraph::new(GraphId::new(16_609));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement should fail")
        .gqlstatus()
}

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(16_606));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    first_output_value(output)
}

fn first_output_value(output: StatementOutput) -> Value {
    let StatementOutput::Rows(table) = output else {
        panic!("statement should produce rows");
    };
    table
        .rows()
        .first()
        .and_then(|row| row.values().first())
        .cloned()
        .expect("one value")
}
