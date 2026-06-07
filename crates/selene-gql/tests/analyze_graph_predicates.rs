//! Analyzer checks for graph-element predicate operand shapes.

use selene_gql::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, EmptyProcedureRegistry,
    GqlType, PipelineStatement, ValueExpr, analyze, parse,
};

fn analyze_one(source: &str) -> Result<AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, None)
}

fn projection_type(analyzed: &AnalyzedStatement, name: &str) -> AnalyzedType {
    let AnalyzedStatementKind::Query(query) = &analyzed.statement else {
        panic!("expected query statement");
    };
    let item = query
        .statements
        .iter()
        .filter_map(|statement| match statement {
            PipelineStatement::Return(clause) => Some(&clause.items),
            PipelineStatement::With(clause) => Some(&clause.items),
            _ => None,
        })
        .flatten()
        .find(|item| {
            item.alias.clone().is_some_and(|alias| alias.as_str() == name)
                || matches!(&item.expr, ValueExpr::Variable { name: value, .. } if value.as_str() == name)
        })
        .unwrap_or_else(|| panic!("projection {name} exists"));
    let id = analyzed
        .expr_ids
        .get(&item.expr)
        .unwrap_or_else(|| panic!("projection {name} has an ExprId"));
    analyzed.expr_types.get(id).clone()
}

fn assert_reference_error(err: AnalysisError, context: &str, expected: &str) {
    match &err {
        AnalysisError::InvalidReference { message, .. } => {
            assert!(message.contains(context));
            assert!(message.contains(expected));
        }
        other => panic!("expected {context} reference error, got {other:?}"),
    }
    assert_eq!(err.gqlstatus().as_str(), "42002");
}

#[test]
fn property_exists_accepts_singleton_element_variables() {
    let node = analyze_one("MATCH (n) RETURN PROPERTY_EXISTS(n, 'name') AS has_name")
        .expect("node variable target analyzes");
    assert_eq!(
        projection_type(&node, "has_name"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );

    let edge = analyze_one("MATCH ()-[r:K]->() RETURN PROPERTY_EXISTS(r, 'weight') AS ok")
        .expect("edge variable target analyzes");
    assert_eq!(
        projection_type(&edge, "ok"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
}

#[test]
fn property_exists_rejects_non_element_variable_references() {
    for source in [
        "MATCH (a)-[r:K*1..2]->(b) RETURN PROPERTY_EXISTS(r, 'weight')",
        "RETURN PROPERTY_EXISTS({foo: 1}, 'foo') AS ok",
        "RETURN PROPERTY_EXISTS(1, 'foo') AS ok",
        "MATCH (n) RETURN PROPERTY_EXISTS(n.payload, 'foo') AS ok",
        "MATCH (n) WITH n AS x RETURN PROPERTY_EXISTS(x, 'name') AS ok",
    ] {
        let err = analyze_one(source).expect_err("invalid PROPERTY_EXISTS target rejects");
        assert_reference_error(
            err,
            "PROPERTY_EXISTS target",
            "singleton node or edge variable reference",
        );
    }
}

#[test]
fn graph_identity_predicates_accept_singleton_element_variables() {
    let nodes =
        analyze_one("MATCH (a), (b) RETURN ALL_DIFFERENT(a, b) AS diff, SAME(a, b) AS same")
            .expect("node variable arguments analyze");
    assert_eq!(
        projection_type(&nodes, "diff"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
    assert_eq!(
        projection_type(&nodes, "same"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );

    let edges = analyze_one("MATCH ()-[r]->(), ()-[s]->() RETURN ALL_DIFFERENT(r, s) AS diff")
        .expect("edge variable arguments analyze");
    assert_eq!(
        projection_type(&edges, "diff"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
}

#[test]
fn graph_identity_predicates_reject_non_element_variable_references() {
    for (source, context) in [
        (
            "RETURN ALL_DIFFERENT(1, 2) AS ok",
            "ALL_DIFFERENT arguments",
        ),
        ("RETURN SAME({foo: 1}, {foo: 1}) AS ok", "SAME arguments"),
        (
            "MATCH (n) RETURN ALL_DIFFERENT(n.payload, n) AS ok",
            "ALL_DIFFERENT arguments",
        ),
        (
            "MATCH (n) WITH n AS x RETURN SAME(x, x) AS ok",
            "SAME arguments",
        ),
        (
            "MATCH (a)-[r:K*1..2]->(b) RETURN SAME(r, b) AS ok",
            "SAME arguments",
        ),
    ] {
        let err = analyze_one(source).expect_err("invalid graph predicate argument rejects");
        assert_reference_error(err, context, "singleton node or edge variable references");
    }
}

#[test]
fn source_destination_predicates_accept_node_edge_variables() {
    let analyzed = analyze_one(
        "MATCH (a)-[e]->(b) RETURN a IS SOURCE OF e AS source, \
         b IS DESTINATION OF e AS destination",
    )
    .expect("node/edge endpoint predicates analyze");
    assert_eq!(
        projection_type(&analyzed, "source"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
    assert_eq!(
        projection_type(&analyzed, "destination"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
}

#[test]
fn source_destination_predicates_reject_non_node_edge_references() {
    for (source, predicate) in [
        ("RETURN 1 IS SOURCE OF 2 AS ok", "IS SOURCE OF"),
        (
            "MATCH ()-[e]->() RETURN e IS SOURCE OF e AS ok",
            "IS SOURCE OF",
        ),
        ("MATCH (n) RETURN n IS SOURCE OF n AS ok", "IS SOURCE OF"),
        (
            "MATCH (n)-[e]->() RETURN n.payload IS SOURCE OF e AS ok",
            "IS SOURCE OF",
        ),
        (
            "MATCH (n)-[e]->() WITH n AS x, e RETURN x IS SOURCE OF e AS ok",
            "IS SOURCE OF",
        ),
        (
            "MATCH (a)-[r:K*1..2]->(b) RETURN a IS SOURCE OF r AS ok",
            "IS SOURCE OF",
        ),
        (
            "MATCH (n)-[e]->() RETURN n IS DESTINATION OF n AS ok",
            "IS DESTINATION OF",
        ),
    ] {
        let err = analyze_one(source).expect_err("invalid endpoint predicate reference rejects");
        assert_reference_error(err, predicate, "variable reference");
    }
}

#[test]
fn directed_labeled_predicates_accept_singleton_element_variables() {
    let analyzed = analyze_one(
        "MATCH (n:Person)-[e:KNOWS]->() \
         RETURN e IS DIRECTED AS directed, n IS LABELED :Person AS node_label, \
         e IS LABELED :KNOWS AS edge_label",
    )
    .expect("directed/labeled predicates analyze");
    assert_eq!(
        projection_type(&analyzed, "directed"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
    assert_eq!(
        projection_type(&analyzed, "node_label"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
    assert_eq!(
        projection_type(&analyzed, "edge_label"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
}

#[test]
fn directed_predicate_rejects_non_edge_variable_references() {
    for source in [
        "RETURN 1 IS DIRECTED AS ok",
        "MATCH (n) RETURN n IS DIRECTED AS ok",
        "MATCH (n)-[e]->() RETURN e.weight IS DIRECTED AS ok",
        "MATCH ()-[e]->() WITH e AS x RETURN x IS DIRECTED AS ok",
        "MATCH (a)-[r:K*1..2]->(b) RETURN r IS DIRECTED AS ok",
    ] {
        let err = analyze_one(source).expect_err("invalid IS DIRECTED operand rejects");
        assert_reference_error(err, "IS DIRECTED", "singleton edge variable reference");
    }
}

#[test]
fn labeled_predicate_rejects_non_element_variable_references() {
    for source in [
        "RETURN 1 IS LABELED :Person AS ok",
        "MATCH (n) RETURN n.name IS LABELED :Person AS ok",
        "MATCH (n) WITH n AS x RETURN x IS LABELED :Person AS ok",
        "MATCH (a)-[r:K*1..2]->(b) RETURN r IS LABELED :K AS ok",
    ] {
        let err = analyze_one(source).expect_err("invalid IS LABELED operand rejects");
        assert_reference_error(
            err,
            "IS LABELED",
            "singleton node or edge variable reference",
        );
    }
}
