//! ISO open reference value type form coverage.

use selene_core::{EdgeId, GraphId, NodeId, Value, db_string};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, GqlType, IsCheckKind, ParserError, PipelineStatement,
    Session, Statement, StatementOutput, ValueExpr, ast::format_read_statement, parse,
};
use selene_graph::SharedGraph;
use selene_profile::FeatureId;

#[test]
fn open_node_and_edge_reference_type_forms_parse_to_ast() {
    for source in [
        "RETURN NULL IS TYPED NODE AS ok",
        "RETURN NULL IS TYPED ANY NODE AS ok",
        "RETURN NULL IS TYPED VERTEX AS ok",
        "RETURN NULL IS TYPED ANY VERTEX AS ok",
    ] {
        assert_eq!(typed_type(source), GqlType::NodeRef, "{source}");
    }

    for source in [
        "RETURN NULL IS TYPED EDGE AS ok",
        "RETURN NULL IS TYPED ANY EDGE AS ok",
        "RETURN NULL IS TYPED RELATIONSHIP AS ok",
        "RETURN NULL IS TYPED ANY RELATIONSHIP AS ok",
    ] {
        assert_eq!(typed_type(source), GqlType::EdgeRef, "{source}");
    }
}

#[test]
fn open_graph_reference_type_forms_report_gv60_unsupported() {
    for source in [
        "RETURN NULL IS TYPED GRAPH AS ok",
        "RETURN NULL IS TYPED ANY GRAPH AS ok",
        "RETURN NULL IS TYPED PROPERTY GRAPH AS ok",
        "RETURN NULL IS TYPED ANY PROPERTY GRAPH AS ok",
    ] {
        let err = selene_gql::parse(source)
            .expect_err("GRAPH reference type remains runtime-unsupported");
        let ParserError::UnsupportedFeature { feature_id, .. } = err else {
            panic!("{source} should report unsupported GV60, got {err:?}");
        };
        assert_eq!(feature_id, FeatureId::GV60, "{source}");
    }
}

#[test]
fn open_reference_type_keywords_require_boundaries() {
    for source in [
        "RETURN NULL IS TYPED ANYNODE AS ok",
        "RETURN NULL IS TYPED ANYVERTEX AS ok",
        "RETURN NULL IS TYPED ANYEDGE AS ok",
        "RETURN NULL IS TYPED ANYRELATIONSHIP AS ok",
        "RETURN NULL IS TYPED GRAPHARRAY AS ok",
        "RETURN NULL IS TYPED GRAPHNOT NULL AS ok",
        "RETURN NULL IS TYPED ANYGRAPH AS ok",
        "RETURN NULL IS TYPED PROPERTYGRAPH AS ok",
        "RETURN NULL IS TYPED ANYPROPERTY GRAPH AS ok",
        "RETURN NULL IS TYPED ANY PROPERTYGRAPH AS ok",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn open_reference_type_keywords_accept_whitespace_boundaries() {
    assert_eq!(
        typed_type("RETURN NULL IS TYPED ANY NODE AS ok"),
        GqlType::NodeRef
    );
    assert_eq!(
        typed_type("RETURN NULL IS TYPED ANY RELATIONSHIP AS ok"),
        GqlType::EdgeRef
    );

    for source in [
        "RETURN NULL IS TYPED ANY GRAPH AS ok",
        "RETURN NULL IS TYPED PROPERTY GRAPH AS ok",
        "RETURN NULL IS TYPED ANY PROPERTY GRAPH AS ok",
    ] {
        let err = selene_gql::parse(source)
            .expect_err("GRAPH reference type remains runtime-unsupported");
        let ParserError::UnsupportedFeature { feature_id, .. } = err else {
            panic!("{source} should report unsupported GV60, got {err:?}");
        };
        assert_eq!(feature_id, FeatureId::GV60, "{source}");
    }
}

#[test]
fn open_reference_type_keywords_accept_comment_boundaries() {
    assert_eq!(
        typed_type("RETURN NULL IS TYPED ANY /* boundary */ NODE AS ok"),
        GqlType::NodeRef
    );
    assert_eq!(
        typed_type("RETURN NULL IS TYPED ANY /* boundary */ RELATIONSHIP AS ok"),
        GqlType::EdgeRef
    );

    for source in [
        "RETURN NULL IS TYPED ANY /* boundary */ GRAPH AS ok",
        "RETURN NULL IS TYPED PROPERTY /* boundary */ GRAPH AS ok",
        "RETURN NULL IS TYPED ANY /* boundary */ PROPERTY /* boundary */ GRAPH AS ok",
    ] {
        let err = selene_gql::parse(source)
            .expect_err("GRAPH reference type remains runtime-unsupported");
        let ParserError::UnsupportedFeature { feature_id, .. } = err else {
            panic!("{source} should report unsupported GV60, got {err:?}");
        };
        assert_eq!(feature_id, FeatureId::GV60, "{source}");
    }
}

#[test]
fn open_graph_element_reference_type_forms_format_canonically() {
    for (source, expected) in [
        (
            "RETURN NULL IS TYPED ANY VERTEX AS ok",
            "RETURN null IS TYPED NODE AS ok",
        ),
        (
            "RETURN NULL IS TYPED ANY RELATIONSHIP AS ok",
            "RETURN null IS TYPED EDGE AS ok",
        ),
        (
            "RETURN NULL IS TYPED RECORD{src_ref :: ANY NODE, dst_ref :: RELATIONSHIP} AS ok",
            "RETURN null IS TYPED RECORD{src_ref :: NODE, dst_ref :: EDGE} AS ok",
        ),
    ] {
        let statement =
            parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
        let formatted = format_read_statement(&statement).expect("statement formats");
        assert_eq!(formatted, expected);
        parse(&formatted).unwrap_or_else(|error| panic!("{formatted} should reparse: {error:?}"));
    }
}

#[test]
fn open_graph_element_reference_predicates_enforce_membership() {
    let graph = SharedGraph::new(GraphId::new(16_620));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("node").expect("valid parameter name"),
        Value::NodeRef(NodeId::new(7)),
    );
    session.bind_parameter(
        db_string("edge").expect("valid parameter name"),
        Value::EdgeRef(EdgeId::new(9)),
    );
    session.bind_parameter(
        db_string("missing").expect("valid parameter name"),
        Value::Null,
    );

    assert_eq!(
        first_value(&mut session, "RETURN $node IS TYPED NODE AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value(&mut session, "RETURN $node IS TYPED EDGE AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value(&mut session, "RETURN $edge IS TYPED RELATIONSHIP AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value(&mut session, "RETURN $missing IS TYPED NODE AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value(&mut session, "RETURN $missing IS TYPED NODE NOT NULL AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn open_graph_element_references_compose_inside_closed_dynamic_unions() {
    let graph = SharedGraph::new(GraphId::new(16_621));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("node").expect("valid parameter name"),
        Value::NodeRef(NodeId::new(7)),
    );
    session.bind_parameter(
        db_string("edge").expect("valid parameter name"),
        Value::EdgeRef(EdgeId::new(9)),
    );
    session.bind_parameter(
        db_string("text").expect("valid parameter name"),
        Value::String(db_string("not a graph element").expect("valid string")),
    );

    for source in [
        "RETURN $node IS TYPED NODE | EDGE AS ok",
        "RETURN $edge IS TYPED NODE | EDGE AS ok",
    ] {
        assert_eq!(
            first_value(&mut session, source),
            Value::Bool(true),
            "{source}"
        );
    }
    assert_eq!(
        first_value(&mut session, "RETURN $text IS TYPED NODE | EDGE AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn open_graph_element_reference_typed_parameters_validate_runtime_values() {
    let graph = SharedGraph::new(GraphId::new(16_622));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("node").expect("valid parameter name"),
        Value::NodeRef(NodeId::new(7)),
    );
    session.bind_parameter(
        db_string("edge").expect("valid parameter name"),
        Value::EdgeRef(EdgeId::new(9)),
    );

    assert_eq!(
        first_value(&mut session, "RETURN $node :: NODE AS value"),
        Value::NodeRef(NodeId::new(7))
    );
    assert_eq!(
        first_value(&mut session, "RETURN $edge :: ANY RELATIONSHIP AS value"),
        Value::EdgeRef(EdgeId::new(9))
    );

    session.bind_parameter(
        db_string("node").expect("valid parameter name"),
        Value::String(db_string("not a node").expect("valid string")),
    );
    let err = session
        .execute_source("RETURN $node :: NODE AS value", &EmptyProcedureRegistry)
        .expect_err("typed NODE parameter rejects non-node value");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "STRING",
            ..
        } if name.as_str() == "node" && expected == "NODE"
    ));
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

fn assert_syntax_error(source: &str) {
    let err = parse(source).expect_err("source should reject");
    assert!(
        matches!(err, ParserError::SyntaxError { .. }),
        "{source:?} should reject as syntax, got {err:?}"
    );
}

fn first_value(session: &mut Session<'_>, source: &str) -> Value {
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("{source} should produce rows");
    };
    table
        .rows()
        .first()
        .and_then(|row| row.values().first())
        .cloned()
        .expect("one value")
}
