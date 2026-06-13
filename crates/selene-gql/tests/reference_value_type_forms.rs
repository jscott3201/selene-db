//! ISO open reference value type form coverage.

use selene_core::feature_register::FeatureId;
use selene_gql::{
    GqlType, IsCheckKind, ParserError, PipelineStatement, Statement, ValueExpr,
    ast::format_read_statement, parse,
};

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
        let err = parse(source).expect_err("GRAPH reference type remains unclaimed");
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
