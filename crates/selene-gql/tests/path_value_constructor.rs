//! `PATH[...]` value-constructor integration tests.

use selene_core::{
    DbString, EdgeDirection, EdgeId, GraphId, LabelSet, NodeId, PathSegment, PropertyMap, Value,
    feature_register::FeatureId,
};
use selene_gql::{
    EmptyProcedureRegistry, PipelineStatement, Session, StatementOutput, ValueExpr, analyze,
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn build_chain_graph() -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(62_001));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let a = mutator
            .create_node(LabelSet::single(db_string("A")), PropertyMap::new())
            .unwrap();
        let b = mutator
            .create_node(LabelSet::single(db_string("B")), PropertyMap::new())
            .unwrap();
        let c = mutator
            .create_node(LabelSet::single(db_string("C")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(db_string("K"), a, b, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(db_string("K"), b, c, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }
    graph
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn single_value(graph: &SharedGraph, source: &str) -> Value {
    let mut session = Session::new(graph);
    let table = rows(
        session
            .execute_source(source, &EmptyProcedureRegistry)
            .expect("query succeeds"),
    );
    assert_eq!(table.row_count(), 1, "{source}");
    table.rows()[0].values()[0].clone()
}

fn status(graph: &SharedGraph, source: &str) -> String {
    let mut session = Session::new(graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("query errors")
        .gqlstatus()
        .to_string()
}

#[test]
fn path_constructor_parses_as_dedicated_ast_node() {
    let statement = parse("MATCH (n) RETURN PATH[n] AS p").unwrap();
    let selene_gql::Statement::Query(pipeline) = statement else {
        panic!("expected query");
    };
    let PipelineStatement::Return(clause) = &pipeline.statements[1] else {
        panic!("expected RETURN");
    };
    let ValueExpr::PathConstructor { elements, .. } = &clause.items[0].expr else {
        panic!("expected path constructor");
    };
    assert_eq!(elements.len(), 1);
}

#[test]
fn path_constructor_formats_as_keyword_bracket_syntax() {
    let parsed = parse("MATCH (n) RETURN path[n] AS p").unwrap();
    let formatted = format_read_statement(&parsed).expect("formats");
    assert_eq!(formatted, "MATCH (n)\nRETURN PATH[n] AS p");
    let reparsed = parse(&formatted).expect("formatted source reparses");
    assert!(structurally_eq(&parsed, &reparsed));
}

#[test]
fn path_constructor_builds_forward_path() {
    let graph = build_chain_graph();
    let value = single_value(
        &graph,
        "MATCH (a:A)-[e:K]->(b:B)-[f:K]->(c:C) RETURN PATH[a, e, b, f, c] AS p",
    );
    let Value::Path(path) = value else {
        panic!("expected path value");
    };
    assert_eq!(path.graph, GraphId::new(62_001));
    assert_eq!(path.start, NodeId::new(1));
    assert_eq!(
        path.segments.as_slice(),
        &[
            PathSegment {
                edge: EdgeId::new(1),
                direction: EdgeDirection::Outgoing,
                node: NodeId::new(2),
            },
            PathSegment {
                edge: EdgeId::new(2),
                direction: EdgeDirection::Outgoing,
                node: NodeId::new(3),
            },
        ]
    );
}

#[test]
fn path_constructor_builds_reverse_traversal_step() {
    let graph = build_chain_graph();
    let value = single_value(&graph, "MATCH (a:A)-[e:K]->(b:B) RETURN PATH[b, e, a] AS p");
    let Value::Path(path) = value else {
        panic!("expected path value");
    };
    assert_eq!(path.start, NodeId::new(2));
    assert_eq!(
        path.segments.as_slice(),
        &[PathSegment {
            edge: EdgeId::new(1),
            direction: EdgeDirection::Incoming,
            node: NodeId::new(1),
        }]
    );
}

#[test]
fn path_constructor_reports_malformed_path_for_null_or_disconnected_elements() {
    let graph = build_chain_graph();
    assert_eq!(status(&graph, "RETURN PATH[null] AS p"), "22G0Z");
    assert_eq!(
        status(
            &graph,
            "MATCH (a:A)-[e:K]->(b:B), (c:C) RETURN PATH[a, e, c] AS p"
        ),
        "22G0Z"
    );
}

#[test]
fn path_constructor_rejects_statically_known_non_reference_elements() {
    let statement = parse("RETURN PATH[1] AS p").expect("source parses");
    let err =
        analyze(statement, &EmptyProcedureRegistry, None).expect_err("source should not analyze");
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}

#[test]
fn path_constructor_records_path_construction_features() {
    let statement = parse("MATCH (n) RETURN PATH[n] AS p").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(features.contains(&FeatureId::GE06), "observed {features:?}");
    assert!(features.contains(&FeatureId::GV55), "observed {features:?}");
}
