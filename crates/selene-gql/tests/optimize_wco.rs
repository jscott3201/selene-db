//! BRIEF-29 WCO marker optimizer tests.

use selene_gql::{EmptyProcedureRegistry, JoinTree, analyze, optimize, parse, plan};

fn optimized_one(source: &str) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    optimize(plan, &selene_gql::OptimizeContext::default())
}

#[test]
fn marks_simple_cycle_as_wco_and_adds_node_ordering() {
    let plan = optimized_one("MATCH (a)-[:K]->(b)-[:K]->(c)-[:K]->(a) RETURN a, b, c");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");

    let JoinTree::WorstCaseOptimal {
        intersection,
        node_id_ordering,
    } = &pattern.join_tree
    else {
        panic!("expected WCO marker, got {:?}", pattern.join_tree);
    };
    assert_eq!(intersection.len(), 1);
    assert!(!node_id_ordering.is_empty());
}

#[test]
fn leaves_acyclic_chain_unchanged() {
    let plan = optimized_one("MATCH (a)-[:K]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");

    assert!(!matches!(
        pattern.join_tree,
        JoinTree::WorstCaseOptimal { .. }
    ));
}

#[test]
fn sentinel_wco_join_snapshot() {
    let plan = optimized_one("MATCH (a)-[:K]->(b)-[:K]->(c)-[:K]->(a) RETURN a, b, c");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let summary = match &pattern.join_tree {
        JoinTree::WorstCaseOptimal {
            intersection,
            node_id_ordering,
        } => format!(
            "join=wco\nintersection={}\nnode_id_ordering={}",
            intersection.len(),
            node_id_ordering.len()
        ),
        other => format!("unexpected={other:?}"),
    };

    insta::assert_snapshot!(summary, @r###"
join=wco
intersection=1
node_id_ordering=2
"###);
}
