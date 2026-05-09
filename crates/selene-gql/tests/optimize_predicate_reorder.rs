//! BRIEF-29 predicate-reorder optimizer tests.

use selene_gql::{
    BinaryOp, EmptyProcedureRegistry, JoinTree, NodeOrEdgeScan, ValueExpr, analyze, optimize,
    parse, plan,
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
fn equality_sorts_before_inequality_in_scan_bucket() {
    let plan = optimized_one("MATCH (n:P) WHERE n.x <> 1 AND n.y = 2 RETURN n");
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ops = scan
        .property_predicates
        .iter()
        .map(|pred| match &pred.expr {
            ValueExpr::BinaryOp { op, .. } => *op,
            other => panic!("unexpected predicate {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(ops, vec![BinaryOp::Eq, BinaryOp::Ne]);
}

#[test]
fn sentinel_predicate_reorder_snapshot() {
    let plan = optimized_one("MATCH (n:P) WHERE n.x <> 1 AND n.y = 2 RETURN n");
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    let summary = scan
        .property_predicates
        .iter()
        .map(|pred| match &pred.expr {
            ValueExpr::BinaryOp {
                op: BinaryOp::Eq, ..
            } => "eq",
            ValueExpr::BinaryOp {
                op: BinaryOp::Ne, ..
            } => "ne",
            other => panic!("unexpected predicate {other:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n");

    insta::assert_snapshot!(summary, @r###"
eq
ne
"###);
}
