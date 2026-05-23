//! Closed-graph static schema validation tests.

use selene_core::{IStr, LabelSet, PropertyValueType, intern};
use selene_gql::{
    AnalysisError, AnalyzedStatement, EdgeDirection, EdgePattern, EmptyProcedureRegistry,
    GraphPattern, InsertStatement, LabelExpr, Literal, MutationPipeline, MutationStatement,
    NodePattern, NonEmpty, PatternElement, SourceSpan, Statement, ValueExpr, analyze, parse,
};
use selene_graph::{
    EdgeEndpointDef, EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef, ValidationMode,
};
use selene_testing::{
    mentions_one_of_graph_type, person_company_graph_type, person_only_graph_type,
};

fn istr(value: &str) -> IStr {
    intern(value).expect("test strings fit interner")
}

fn labels(values: &[&str]) -> LabelSet {
    values.iter().map(|value| istr(value)).collect()
}

fn property(name: &str, value_type: PropertyValueType, required: bool) -> PropertyTypeDef {
    PropertyTypeDef {
        name: istr(name),
        value_type,
        list_element_type: None,
        required,
        default: None,
        immutable: false,
    }
}

fn analyze_source(
    source: &str,
    schema: Option<&GraphTypeDef>,
) -> Result<AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, schema)
}

fn analyze_with_schema(
    source: &str,
    graph_type: &GraphTypeDef,
) -> Result<AnalyzedStatement, AnalysisError> {
    analyze_source(source, Some(graph_type))
}

fn schema_error(source: &str, graph_type: &GraphTypeDef) -> AnalysisError {
    analyze_with_schema(source, graph_type).expect_err("schema validation rejects input")
}

fn label_expr(label: &str) -> Option<LabelExpr> {
    Some(LabelExpr::Single(istr(label)))
}

fn string_expr(value: &str, span: SourceSpan) -> ValueExpr {
    ValueExpr::Literal(Literal::String(istr(value), span))
}

fn node(name: &str, label: &str, name_value: &str, span: SourceSpan) -> PatternElement {
    PatternElement::Node(NodePattern {
        binding: Some(istr(name)),
        label_expr: label_expr(label),
        properties: vec![(istr("name"), string_expr(name_value, span))],
        inline_where: None,
        span,
    })
}

fn edge(label: &str, direction: EdgeDirection, span: SourceSpan) -> PatternElement {
    PatternElement::Edge(EdgePattern {
        binding: None,
        direction,
        label_expr: label_expr(label),
        properties: Vec::new(),
        quantifier: None,
        inline_where: None,
        span,
    })
}

fn ambiguous_property_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("fixture.ambiguous"),
        node_types: vec![
            NodeTypeDef {
                name: istr("Person"),
                key_labels: LabelSet::single(istr("Person")),
                properties: vec![
                    property("name", PropertyValueType::String, true),
                    property("flag", PropertyValueType::String, false),
                ],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("ActivePerson"),
                key_labels: labels(&["Person", "Active"]),
                properties: vec![
                    property("name", PropertyValueType::String, true),
                    property("flag", PropertyValueType::Int, false),
                ],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: Vec::new(),
    }
    .validate()
    .expect("fixture graph type is valid")
}

fn duplicate_edge_label_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("fixture.duplicate_edge_label"),
        node_types: vec![
            NodeTypeDef {
                name: istr("Person"),
                key_labels: LabelSet::single(istr("Person")),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("Company"),
                key_labels: LabelSet::single(istr("Company")),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![
            EdgeTypeDef {
                name: istr("WorksAt"),
                label: istr("REL"),
                source_node_type: EdgeEndpointDef::NodeType(0),
                target_node_type: EdgeEndpointDef::NodeType(1),
                properties: vec![property("since", PropertyValueType::Int, false)],
                validation_mode: ValidationMode::Strict,
            },
            EdgeTypeDef {
                name: istr("Knows"),
                label: istr("REL"),
                source_node_type: EdgeEndpointDef::NodeType(0),
                target_node_type: EdgeEndpointDef::NodeType(0),
                properties: vec![property("strength", PropertyValueType::Int, false)],
                validation_mode: ValidationMode::Strict,
            },
        ],
    }
    .validate()
    .expect("fixture graph type is valid")
}

fn label_transition_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("fixture.label_transition"),
        node_types: vec![
            NodeTypeDef {
                name: istr("Person"),
                key_labels: LabelSet::single(istr("Person")),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("ActivePerson"),
                key_labels: labels(&["Person", "Active"]),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("SeniorPerson"),
                key_labels: labels(&["Person", "Senior"]),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: istr("VisitorPerson"),
                key_labels: labels(&["Person", "Visitor"]),
                properties: vec![property("name", PropertyValueType::String, true)],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: Vec::new(),
    }
    .validate()
    .expect("fixture graph type is valid")
}

#[test]
fn schema_absent_skips_closed_graph_checks() {
    analyze_source("INSERT (n { extra: 1 })", None).expect("open graph accepts insert");
}

#[test]
fn validates_insert_node_and_edge_against_schema() {
    let graph_type = person_company_graph_type();
    analyze_with_schema("INSERT (n:Person { name: 'Alice' })", &graph_type).expect("valid node");
    let source = concat!(
        "INSERT ",
        "(a:Person { name: 'Alice' })",
        "-[:WORKS_AT { since: 2026 }]->",
        "(b:Company { name: 'Acme' })"
    );
    analyze_with_schema(source, &graph_type).expect("valid edge");
}

#[test]
fn rejects_unknown_node_type() {
    let graph_type = person_company_graph_type();
    let error = schema_error("INSERT (n:Project { name: 'X' })", &graph_type);
    assert_eq!(error.gqlstatus().as_str(), "G2000");
    assert!(matches!(error, AnalysisError::SchemaUnknownNodeType { .. }));
}

#[test]
fn rejects_unknown_edge_type() {
    let graph_type = person_company_graph_type();
    let error = schema_error(
        "INSERT (a:Person { name: 'A' })-[:LIKES]->(b:Company { name: 'B' })",
        &graph_type,
    );
    assert!(matches!(error, AnalysisError::SchemaUnknownEdgeType { .. }));
}

#[test]
fn rejects_unknown_label_on_undirected_insert_edge() {
    let graph_type = person_company_graph_type();
    let pattern = GraphPattern {
        path_binding: None,
        elements: vec![
            node("a", "Person", "A", SourceSpan::new(0, 1)),
            edge("LIKES", EdgeDirection::Undirected, SourceSpan::new(1, 1)),
            node("b", "Company", "B", SourceSpan::new(2, 1)),
        ],
        span: SourceSpan::new(0, 3),
    };
    let statement = Statement::Mutate(MutationPipeline {
        statements: NonEmpty::try_from_vec(vec![MutationStatement::Insert(InsertStatement {
            patterns: vec![pattern],
            span: SourceSpan::new(0, 3),
        })])
        .expect("non-empty"),
        terminator: None,
        span: SourceSpan::new(0, 3),
    });
    let error = analyze(statement, &EmptyProcedureRegistry, Some(&graph_type))
        .expect_err("unknown edge label rejects before undirected defer");
    assert!(matches!(error, AnalysisError::SchemaUnknownEdgeType { .. }));
}

#[test]
fn rejects_unknown_label_when_endpoints_are_dynamic() {
    let graph_type = person_company_graph_type();
    let error = schema_error("MATCH (a), (b) INSERT (a)-[:LIKES]->(b)", &graph_type);
    assert!(matches!(error, AnalysisError::SchemaUnknownEdgeType { .. }));
}

#[test]
fn rejects_static_edge_endpoint_mismatch() {
    let graph_type = person_company_graph_type();
    let error = schema_error(
        "INSERT (a:Company { name: 'A' })-[:WORKS_AT]->(b:Person { name: 'B' })",
        &graph_type,
    );
    assert!(matches!(
        error,
        AnalysisError::SchemaEdgeEndpointMismatch { .. }
    ));
}

#[test]
fn reused_insert_node_uses_exact_endpoint_lookup() {
    let graph_type = person_company_graph_type();
    let error = schema_error(
        concat!(
            "INSERT (n:Person { name: 'A' }) ",
            "INSERT (n)-[:WORKS_AT]->(b:Person { name: 'B' })"
        ),
        &graph_type,
    );
    assert!(matches!(
        error,
        AnalysisError::SchemaEdgeEndpointMismatch { .. }
    ));
}

#[test]
fn rejects_undeclared_property_on_insert_and_set() {
    let graph_type = person_company_graph_type();
    let insert_error = schema_error("INSERT (n:Person { name: 'Alice', age: 42 })", &graph_type);
    assert!(matches!(
        insert_error,
        AnalysisError::SchemaUndeclaredProperty { .. }
    ));

    let set_error = schema_error("MATCH (n:Person) SET n.age = 42", &graph_type);
    assert!(matches!(
        set_error,
        AnalysisError::SchemaUndeclaredProperty { .. }
    ));
}

#[test]
fn match_then_match_with_labels_uses_refined_labels_for_set_validation() {
    let graph_type = person_company_graph_type();
    let error = schema_error("MATCH (n) MATCH (n:Person) SET n.foo = 1", &graph_type);
    assert!(matches!(
        error,
        AnalysisError::SchemaUndeclaredProperty { .. }
    ));
}

#[test]
fn rejects_property_type_mismatch_on_insert_and_set() {
    let graph_type = person_company_graph_type();
    let insert_error = schema_error("INSERT (n:Person { name: 42 })", &graph_type);
    assert!(matches!(
        insert_error,
        AnalysisError::SchemaPropertyTypeMismatch { .. }
    ));

    let set_error = schema_error("MATCH (n:Person) SET n.name = 42", &graph_type);
    assert!(matches!(
        set_error,
        AnalysisError::SchemaPropertyTypeMismatch { .. }
    ));
}

#[test]
fn rejects_duplicate_set_map_keys_with_mismatched_values() {
    let graph_type = person_company_graph_type();
    let error = schema_error(
        "MATCH (n:Person) SET n = { name: 'A', name: 1 }",
        &graph_type,
    );
    assert!(matches!(
        error,
        AnalysisError::SchemaPropertyTypeMismatch { .. }
    ));
}

#[test]
fn rejects_missing_and_removed_required_property() {
    let graph_type = person_company_graph_type();
    let missing = schema_error("INSERT (n:Person)", &graph_type);
    assert!(matches!(
        missing,
        AnalysisError::SchemaRequiredPropertyMissing { .. }
    ));

    let removed = schema_error("MATCH (n:Person) REMOVE n.name", &graph_type);
    assert!(matches!(
        removed,
        AnalysisError::SchemaRequiredPropertyRemoved { .. }
    ));
}

#[test]
fn rejects_invalid_insert_label_forms() {
    let graph_type = person_company_graph_type();
    for source in [
        "INSERT (n { name: 'Alice' })",
        "INSERT (n:Person|Company { name: 'Alice' })",
        "INSERT (n:!Person { name: 'Alice' })",
        "INSERT (n:% { name: 'Alice' })",
    ] {
        let error = schema_error(source, &graph_type);
        assert!(matches!(
            error,
            AnalysisError::SchemaInvalidInsertLabelExpr { .. }
        ));
    }
}

#[test]
fn required_insert_property_can_be_supplied_by_later_set() {
    let graph_type = person_only_graph_type();
    analyze_with_schema("INSERT (n:Person) SET n.name = 'Alice'", &graph_type)
        .expect("later SET property satisfies required property");
    analyze_with_schema("INSERT (n:Person) SET n = { name: 'Alice' }", &graph_type)
        .expect("later SET property map satisfies required property");

    let error = schema_error("INSERT (n:Person) SET n :Active", &graph_type);
    assert!(matches!(
        error,
        AnalysisError::SchemaRequiredPropertyMissing { .. }
    ));
}

#[test]
fn analyzer_resolves_schema_properties_by_binding_id() {
    let graph_type = person_only_graph_type();
    analyze_with_schema("INSERT (n:Person) SET n.name = 'Alice'", &graph_type)
        .expect("later SET target satisfies required property through its BindingId");
}

#[test]
fn analyzer_resolves_schema_labels_by_binding_id() {
    let graph_type = person_company_graph_type();
    analyze_with_schema(
        concat!(
            "INSERT (n:Person { name: 'A' }) ",
            "INSERT (n)-[:WORKS_AT]->(b:Company { name: 'B' })"
        ),
        &graph_type,
    )
    .expect("reused INSERT endpoint resolves through the original BindingId");
}

#[test]
fn validates_static_candidate_sets_when_candidates_agree() {
    let graph_type = person_company_graph_type();
    analyze_with_schema("MATCH (n:Person) SET n.name = 'Alice'", &graph_type)
        .expect("all Person candidates agree on name");
    analyze_with_schema("MATCH (n:Person) REMOVE n.nickname", &graph_type)
        .expect("all Person candidates agree nickname is optional");
}

#[test]
fn defers_static_candidate_sets_when_candidates_disagree() {
    let graph_type = ambiguous_property_graph_type();
    analyze_with_schema("MATCH (n:Person) SET n.flag = 1", &graph_type)
        .expect("candidate disagreement defers to runtime");
}

#[test]
fn validates_static_label_transitions_when_all_candidates_have_a_target_type() {
    let graph_type = person_company_graph_type();
    analyze_with_schema("MATCH (n:Person) SET n :Active", &graph_type)
        .expect("Person candidates can transition to declared label set");

    let error = schema_error("MATCH (n:Company) SET n :Active", &graph_type);
    assert!(matches!(error, AnalysisError::SchemaUnknownNodeType { .. }));
}

#[test]
fn rejects_label_transition_when_any_candidate_becomes_invalid() {
    let graph_type = label_transition_graph_type();
    let error = schema_error("MATCH (n:Person) SET n :Visitor", &graph_type);
    assert!(matches!(error, AnalysisError::SchemaUnknownNodeType { .. }));
}

#[test]
fn validates_edge_set_and_remove_when_edge_label_is_unique() {
    let graph_type = person_company_graph_type();
    analyze_with_schema(
        "MATCH (a:Person)-[r:WORKS_AT]->(b:Company) SET r.since = 2026",
        &graph_type,
    )
    .expect("valid edge property set");

    let type_error = schema_error(
        "MATCH (a:Person)-[r:WORKS_AT]->(b:Company) SET r.since = 'soon'",
        &graph_type,
    );
    assert!(matches!(
        type_error,
        AnalysisError::SchemaPropertyTypeMismatch { .. }
    ));

    let undeclared = schema_error(
        "MATCH (a:Person)-[r:WORKS_AT]->(b:Company) SET r.extra = 1",
        &graph_type,
    );
    assert!(matches!(
        undeclared,
        AnalysisError::SchemaUndeclaredProperty { .. }
    ));
}

#[test]
fn rejects_remove_required_edge_label() {
    let graph_type = person_company_graph_type();
    let error = schema_error(
        "MATCH (a:Person)-[r:WORKS_AT]->(b:Company) REMOVE r :WORKS_AT",
        &graph_type,
    );
    assert!(matches!(
        error,
        AnalysisError::SchemaRequiredEdgeLabelRemoved { .. }
    ));
}

#[test]
fn defers_edge_set_when_label_has_multiple_edge_types() {
    let graph_type = duplicate_edge_label_graph_type();
    analyze_with_schema(
        "MATCH (a:Person)-[r:REL]->(b:Company) SET r.extra = 1",
        &graph_type,
    )
    .expect("shared edge label defers property validation");
}

#[test]
fn edge_direction_matrix_checks_right_and_left_but_defers_undirected() {
    let graph_type = person_company_graph_type();
    analyze_with_schema(
        "INSERT (a:Person { name: 'A' })-[:WORKS_AT]->(b:Company { name: 'B' })",
        &graph_type,
    )
    .expect("right edge endpoints match");
    analyze_with_schema(
        "INSERT (b:Company { name: 'B' })<-[:WORKS_AT]-(a:Person { name: 'A' })",
        &graph_type,
    )
    .expect("left edge endpoints match");

    let pattern = GraphPattern {
        path_binding: None,
        elements: vec![
            node("a", "Company", "A", SourceSpan::new(0, 1)),
            edge("WORKS_AT", EdgeDirection::Undirected, SourceSpan::new(1, 1)),
            node("b", "Person", "B", SourceSpan::new(2, 1)),
        ],
        span: SourceSpan::new(0, 3),
    };
    let statement = Statement::Mutate(MutationPipeline {
        statements: NonEmpty::try_from_vec(vec![MutationStatement::Insert(InsertStatement {
            patterns: vec![pattern],
            span: SourceSpan::new(0, 3),
        })])
        .expect("non-empty"),
        terminator: None,
        span: SourceSpan::new(0, 3),
    });
    analyze(statement, &EmptyProcedureRegistry, Some(&graph_type))
        .expect("undirected INSERT edge endpoint check defers to runtime");
}

#[test]
fn schema_validation_smoke_preserves_write_set() {
    let graph_type = person_company_graph_type();
    let analyzed = analyze_with_schema(
        "INSERT (n:Person) SET n.name = 'Alice' RETURN n",
        &graph_type,
    )
    .expect("valid mutation analyzes");
    let write_set = analyzed.write_set.expect("mutation write-set");
    assert_eq!(write_set.entries.len(), 2);
}

#[test]
fn one_of_source_endpoint_accepts_either_declared_member() {
    // BRIEF-131e: analyzer must accept INSERT with either Document OR Comment
    // as the MENTIONS source. `matches_node_type` does the membership check on
    // OneOf at the analyzer layer too.
    let graph_type = mentions_one_of_graph_type();
    analyze_with_schema(
        "INSERT (a:Document { title: 'T' })-[:MENTIONS]->(b:Topic { name: 'N' })",
        &graph_type,
    )
    .expect("Document is in OneOf source set");
    analyze_with_schema(
        "INSERT (a:Comment { body: 'B' })-[:MENTIONS]->(b:Topic { name: 'N' })",
        &graph_type,
    )
    .expect("Comment is in OneOf source set");
}

#[test]
fn one_of_source_endpoint_rejects_non_member() {
    // F11 coverage at the analyzer layer: the rendered endpoint name for an
    // OneOf endpoint is comma-joined member node-type names (per the commit-1
    // endpoint_name OneOf arm). The error must still surface as
    // SchemaEdgeEndpointMismatch.
    let graph_type = mentions_one_of_graph_type();
    let error = schema_error(
        "INSERT (a:Topic { name: 'N1' })-[:MENTIONS]->(b:Topic { name: 'N2' })",
        &graph_type,
    );
    assert!(matches!(
        error,
        AnalysisError::SchemaEdgeEndpointMismatch { ref expected_source, .. }
            if expected_source.contains("Document") && expected_source.contains("Comment")
    ));
}
