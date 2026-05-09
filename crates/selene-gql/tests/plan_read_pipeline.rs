//! BRIEF-26 read-side planner lowering tests.

use selene_gql::{
    AnalyzedStatement, BindingElement, EdgeDirection, EmptyProcedureRegistry, FilterPredicateKind,
    JoinTree, LabelExpr, LimitAmount, PipelineOp, PlannerError, ScanKind, SetOp, analyze, parse,
    plan,
};

fn analyze_one(source: &str) -> AnalyzedStatement {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes")
}

fn plan_one(source: &str) -> selene_gql::ExecutionPlan {
    let analyzed = analyze_one(source);
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn plan_err(source: &str) -> PlannerError {
    let analyzed = analyze_one(source);
    plan(&analyzed, &EmptyProcedureRegistry).expect_err("test input should fail planning")
}

fn variant_names(plan: &selene_gql::ExecutionPlan) -> Vec<&'static str> {
    plan.pipeline
        .iter()
        .map(|op| match op {
            PipelineOp::Filter(_) => "Filter",
            PipelineOp::Project(_) => "Project",
            PipelineOp::Unwind { .. } => "Unwind",
            PipelineOp::OrderBy(_) => "OrderBy",
            PipelineOp::Limit { .. } => "Limit",
            PipelineOp::GroupBy { .. } => "GroupBy",
            PipelineOp::Distinct => "Distinct",
            PipelineOp::Union { .. } => "Union",
            PipelineOp::Chain(_) => "Chain",
            PipelineOp::Call(_) => "Call",
            PipelineOp::Mutation(_) => "Mutation",
        })
        .collect()
}

fn expand(plan: &selene_gql::ExecutionPlan) -> (&selene_gql::EdgeMatch, EdgeDirection) {
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    match &pattern.join_tree {
        JoinTree::Expand {
            edge, direction, ..
        } => (edge, *direction),
        other => panic!("expected expand, got {other:?}"),
    }
}

#[test]
fn lowers_match_return_scan() {
    let plan = plan_one("MATCH (n) RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert_eq!(pattern.bindings.len(), 1);
    assert_eq!(pattern.bindings[0].name.as_str(), "n");
    assert_eq!(pattern.bindings[0].element, BindingElement::Node);
    let JoinTree::Scan(scan) = &pattern.join_tree else {
        panic!("expected scan");
    };
    assert_eq!(scan.kind, ScanKind::Node);
    assert!(scan.binding.is_some());
    assert_eq!(variant_names(&plan), ["Project"]);
    assert_eq!(plan.output_schema.columns[0].name.unwrap().as_str(), "n");
}

#[test]
fn anonymous_node_scan_has_no_binding() {
    let plan = plan_one("MATCH (:Person) RETURN 1");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let JoinTree::Scan(scan) = &pattern.join_tree else {
        panic!("expected scan");
    };
    assert_eq!(scan.binding, None);
    assert!(matches!(
        scan.label_predicate,
        Some(LabelExpr::Single(label)) if label.as_str() == "Person"
    ));
}

#[test]
fn anonymous_edge_endpoints_are_preserved_as_none() {
    let plan = plan_one("MATCH ()-[e]->() RETURN e");
    let (edge, direction) = expand(&plan);
    assert_eq!(direction, EdgeDirection::Right);
    assert!(edge.binding.is_some());
    assert_eq!(edge.left_binding, None);
    assert_eq!(edge.right_binding, None);
}

#[test]
fn direction_matrix_preserves_pattern_direction() {
    let right = plan_one("MATCH (a)-[:K]->(b) RETURN a, b");
    assert_eq!(expand(&right).1, EdgeDirection::Right);

    let left = plan_one("MATCH (a)<-[:K]-(b) RETURN a, b");
    assert_eq!(expand(&left).1, EdgeDirection::Left);

    let undirected = plan_one("MATCH (a)-[:K]-(b) RETURN a, b");
    assert_eq!(expand(&undirected).1, EdgeDirection::Undirected);
}

#[test]
fn optional_match_lowers_to_outer_join() {
    let plan = plan_one("MATCH (a) OPTIONAL MATCH (a)-[:K]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert!(matches!(pattern.join_tree, JoinTree::Outer { .. }));
}

#[test]
fn path_binding_creates_path_plan_placeholder() {
    let plan = plan_one("MATCH p = (a)-[:K]->(b) RETURN p");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert_eq!(pattern.paths.len(), 1);
}

#[test]
fn property_maps_and_inline_where_are_preserved() {
    let plan = plan_one("MATCH (n:Person {age: 42} WHERE n.active) RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let JoinTree::Scan(scan) = &pattern.join_tree else {
        panic!("expected scan");
    };
    assert_eq!(scan.property_predicates.len(), 1);
    assert!(matches!(
        scan.property_predicates[0].kind,
        FilterPredicateKind::PropertyEquals { key, .. } if key.as_str() == "age"
    ));
    assert_eq!(pattern.filters.len(), 1);
}

#[test]
fn preserves_set_op_in_composite_plan() {
    let plan = plan_one("RETURN 1 AS n UNION ALL RETURN 2 AS n");
    let Some(PipelineOp::Union { op, .. }) = plan.pipeline.last() else {
        panic!("expected union op");
    };
    assert_eq!(*op, SetOp::UnionAll);
}

#[test]
fn composite_subplans_keep_pattern_bindings_local() {
    let plan = plan_one("MATCH (n) RETURN n UNION MATCH (m) RETURN m");
    let first_pattern = plan.pattern_plan.as_ref().expect("first pattern");
    assert_eq!(first_pattern.bindings.len(), 1);
    assert_eq!(first_pattern.bindings[0].name.as_str(), "n");
    let Some(PipelineOp::Union { rhs, .. }) = plan.pipeline.last() else {
        panic!("expected union op");
    };
    let rhs_pattern = rhs.pattern_plan.as_ref().expect("rhs pattern");
    assert_eq!(rhs_pattern.bindings.len(), 1);
    assert_eq!(rhs_pattern.bindings[0].name.as_str(), "m");
}

#[test]
fn chained_query_uses_chain_pipeline_op() {
    let plan = plan_one("RETURN 1 AS n NEXT RETURN 2 AS n");
    assert!(matches!(plan.pipeline.last(), Some(PipelineOp::Chain(_))));
}

#[test]
fn limit_parameter_survives_to_plan() {
    let plan = plan_one("RETURN 1 AS n LIMIT $rows");
    let Some(PipelineOp::Limit { count, .. }) = plan.pipeline.last() else {
        panic!("expected limit");
    };
    assert!(matches!(count, LimitAmount::Parameter(name) if name.as_str() == "rows"));
}

#[test]
fn unaliased_derived_projection_column_has_no_name() {
    let plan = plan_one("MATCH (n) RETURN n.name");
    assert_eq!(plan.output_schema.columns.len(), 1);
    assert_eq!(plan.output_schema.columns[0].name, None);
}

#[test]
fn return_star_keeps_visible_binding_columns() {
    let plan = plan_one("MATCH (n) RETURN *");
    assert!(
        plan.output_schema
            .columns
            .iter()
            .any(|column| column.name.is_some_and(|name| name.as_str() == "n"))
    );
}

#[test]
fn order_key_carries_binding_refs() {
    let plan = plan_one("MATCH (n) RETURN n ORDER BY n.name");
    let Some(PipelineOp::OrderBy(keys)) = plan.pipeline.last() else {
        panic!("expected order-by");
    };
    assert_eq!(keys[0].binding_refs.len(), 1);
}

#[test]
fn non_leading_match_is_not_implemented() {
    let err = plan_err("MATCH (a) WITH a AS x MATCH (b) RETURN x, b");
    assert!(matches!(
        err,
        PlannerError::NotImplemented {
            feature: "non-leading MATCH (post-pipeline-boundary pattern)",
            ..
        }
    ));
}

#[test]
fn deferred_pattern_features_have_stable_tags() {
    let cases = [
        (
            "MATCH (a)-[:K*1..2]->(b) RETURN a",
            "variable-length edge patterns (quantifier)",
        ),
        (
            "MATCH ANY (n) RETURN n",
            "MATCH path selector (SHORTEST/ALL/ANY)",
        ),
        (
            "MATCH DIFFERENT EDGES (n) RETURN n",
            "MATCH mode (REPEATABLE ELEMENTS / DIFFERENT EDGES)",
        ),
        (
            "MATCH SIMPLE (n) RETURN n",
            "MATCH path mode (TRAIL/SIMPLE/ACYCLIC)",
        ),
    ];

    for (source, expected) in cases {
        let err = plan_err(source);
        assert!(
            matches!(err, PlannerError::NotImplemented { feature, .. } if feature == expected),
            "{source} should report {expected}, got {err:?}"
        );
    }
}

#[test]
fn planner_errors_emit_xx500() {
    let err = plan_err("MATCH ANY (n) RETURN n");
    assert_eq!(err.gqlstatus().as_str(), "XX500");
}

#[test]
fn sentinel_plan_shape_snapshot() {
    let plan = plan_one("MATCH (n:Person) WHERE n.age > 30 RETURN n.name AS name LIMIT 10");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = match &pattern.join_tree {
        JoinTree::Scan(scan) => scan,
        other => panic!("expected scan, got {other:?}"),
    };
    let summary = format!(
        "scan_binding={}\nscan_label={}\npattern_filters={}\npipeline={:?}\ncolumns={}",
        scan.binding.is_some(),
        matches!(scan.label_predicate, Some(LabelExpr::Single(_))),
        pattern.filters.len(),
        variant_names(&plan),
        plan.output_schema.columns.len()
    );
    insta::assert_snapshot!(summary, @r###"
scan_binding=true
scan_label=true
pattern_filters=1
pipeline=["Project", "Limit"]
columns=1
"###);
}
