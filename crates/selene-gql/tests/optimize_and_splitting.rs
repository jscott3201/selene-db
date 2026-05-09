//! BRIEF-28 AND-splitting optimizer tests.

use selene_gql::{
    BinaryOp, EmptyProcedureRegistry, FilterPredicateKind, JoinTree, NodeOrEdgeScan, PipelineOp,
    ValueExpr, analyze, optimize, parse, plan,
};

fn optimized_one(source: &str) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    optimize(plan, &selene_gql::OptimizeContext::default())
}

fn first_scan(tree: &JoinTree) -> Option<&NodeOrEdgeScan> {
    match tree {
        JoinTree::Scan(scan) => Some(scan),
        JoinTree::Expand { child, .. } => first_scan(child),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            first_scan(left).or_else(|| first_scan(right))
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => None,
        _ => None,
    }
}

#[test]
fn splits_pattern_where_conjunctions() {
    let plan = optimized_one("MATCH (n) WHERE n.a > 1 AND n.b < 5 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = first_scan(&pattern.join_tree).expect("node scan");
    assert!(pattern.filters.is_empty());
    assert_eq!(scan.property_predicates.len(), 2);
    assert!(
        scan.property_predicates
            .iter()
            .all(|pred| pred.kind == FilterPredicateKind::Expression)
    );
}

#[test]
fn flattens_nested_and_in_source_order() {
    let plan = optimized_one("MATCH (n) WHERE n.a > 1 AND n.b < 5 AND n.c = 9 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = first_scan(&pattern.join_tree).expect("node scan");
    assert_eq!(scan.property_predicates.len(), 3);
}

#[test]
fn leaves_non_root_and_unchanged() {
    let plan = optimized_one("MATCH (n) WHERE NOT (n.a > 1 AND n.b < 5) RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = first_scan(&pattern.join_tree).expect("node scan");
    assert_eq!(scan.property_predicates.len(), 1);
    assert!(matches!(
        scan.property_predicates[0].expr,
        ValueExpr::UnaryOp { .. }
    ));
}

#[test]
fn split_binding_refs_are_recomputed_for_each_half() {
    let plan = optimized_one("MATCH (n)-[e]->(m) WHERE e.weight > 1 AND n.age > 2 RETURN n, m");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert!(pattern.filters.is_empty());

    let JoinTree::Expand { edge, .. } = &pattern.join_tree else {
        panic!("expected expand");
    };
    assert_eq!(edge.property_predicates.len(), 1);
}

#[test]
fn splits_pipeline_filter_before_pushdown() {
    let plan = optimized_one("MATCH (n) FILTER n.a > 1 AND n.b < 5 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = first_scan(&pattern.join_tree).expect("node scan");
    assert_eq!(scan.property_predicates.len(), 2);
    assert!(
        plan.pipeline
            .iter()
            .all(|op| !matches!(op, PipelineOp::Filter(_)))
    );
}

#[test]
fn sentinel_and_splitting_snapshot() {
    let plan = optimized_one("MATCH (n) WHERE n.a > 1 AND n.b < 5 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = first_scan(&pattern.join_tree).expect("node scan");
    let summary = scan
        .property_predicates
        .iter()
        .map(|pred| match &pred.expr {
            ValueExpr::BinaryOp { op, .. } => match op {
                BinaryOp::Gt => "gt",
                BinaryOp::Lt => "lt",
                other => panic!("unexpected op {other:?}"),
            },
            other => panic!("unexpected expr {other:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(summary, @r###"
gt
lt
"###);
}
