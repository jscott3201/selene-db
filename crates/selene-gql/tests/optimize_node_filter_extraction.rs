//! BRIEF-29 node-filter extraction tests.

use selene_gql::{
    EmptyProcedureRegistry, JoinTree, NodeOrEdgeScan, analyze, optimize, parse, plan,
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
fn extracts_single_node_filter_to_scan_bucket() {
    let plan = optimized_one("MATCH (n:Person) WHERE n.age > 30 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = first_scan(&pattern.join_tree).expect("node scan");

    assert!(pattern.filters.is_empty());
    assert_eq!(scan.property_predicates.len(), 1);
}

#[test]
fn sentinel_node_filter_extraction_snapshot() {
    let plan = optimized_one("MATCH (n:Person) WHERE n.age > 30 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = first_scan(&pattern.join_tree).expect("node scan");
    let summary = format!(
        "pattern_filters={}\nscan_filters={}",
        pattern.filters.len(),
        scan.property_predicates.len()
    );

    insta::assert_snapshot!(summary, @r###"
pattern_filters=0
scan_filters=1
"###);
}
