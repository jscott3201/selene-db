//! BRIEF-28 expand-filter-pushdown optimizer tests.

use selene_gql::{
    BindingId, EmptyProcedureRegistry, JoinTree, PipelineOp, analyze, optimize, parse, plan,
};

fn planned_one(source: &str) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn optimized_one(source: &str) -> selene_gql::ExecutionPlan {
    optimize(planned_one(source), &selene_gql::OptimizeContext::default())
}

fn binding_by_name(plan: &selene_gql::ExecutionPlan, name: &str) -> BindingId {
    plan.pattern_plan
        .as_ref()
        .expect("pattern plan")
        .bindings
        .iter()
        .find(|binding| binding.name.as_str() == name)
        .expect("binding")
        .binding
}

fn edge_predicate_count(tree: &JoinTree, binding: BindingId) -> usize {
    match tree {
        JoinTree::Scan(_) | JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => 0,
        JoinTree::Expand { child, edge, .. } => {
            edge_predicate_count(child, binding)
                + usize::from(edge.binding == Some(binding)) * edge.property_predicates.len()
        }
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            edge_predicate_count(left, binding) + edge_predicate_count(right, binding)
        }
        _ => 0,
    }
}

#[test]
fn pushes_edge_only_filter_to_expand_edge() {
    let plan = optimized_one("MATCH (a)-[e:KNOWS]->(b) FILTER e.since > 2020 RETURN a, b");
    let binding = binding_by_name(&plan, "e");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert!(pattern.filters.is_empty());
    assert_eq!(edge_predicate_count(&pattern.join_tree, binding), 1);
}

#[test]
fn recurses_through_hash_join() {
    let plan = optimized_one("MATCH (a)-[e]->(b), (c)-[f]->(d) FILTER f.weight > 1 RETURN a");
    let binding = binding_by_name(&plan, "f");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert!(matches!(pattern.join_tree, JoinTree::HashJoin { .. }));
    assert_eq!(edge_predicate_count(&pattern.join_tree, binding), 1);
}

#[test]
fn skips_pushdown_under_outer_right() {
    // Pushing `e.weight > 1` into the optional-side `Expand` would evaluate
    // the predicate before null-extension, dropping unmatched-`a` rows that
    // a post-OPTIONAL FILTER must keep. The predicate must stay in
    // `pattern.filters` (which runs after null-extension).
    let plan = optimized_one("MATCH (a) OPTIONAL MATCH (a)-[e]->(b) FILTER e.weight > 1 RETURN a");
    let binding = binding_by_name(&plan, "e");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert!(matches!(pattern.join_tree, JoinTree::Outer { .. }));
    assert_eq!(edge_predicate_count(&pattern.join_tree, binding), 0);
    assert_eq!(pattern.filters.len(), 1);
}

#[test]
fn pushes_into_outer_left_subtree() {
    // The preserved (left) side of `Outer` is a safe pushdown target —
    // pushing into a left-side `Expand` doesn't change which rows survive
    // null-extension on the right side.
    let plan = optimized_one(
        "MATCH (a)-[e]->(b) OPTIONAL MATCH (b)-[f]->(c) FILTER e.weight > 1 RETURN a",
    );
    let binding = binding_by_name(&plan, "e");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert!(matches!(pattern.join_tree, JoinTree::Outer { .. }));
    assert_eq!(edge_predicate_count(&pattern.join_tree, binding), 1);
    assert!(pattern.filters.is_empty());
}

#[test]
fn worst_case_optimal_intersection_is_opaque() {
    let mut plan = planned_one("MATCH (a)-[e]->(b) FILTER e.weight > 1 RETURN a");
    let PipelineOp::Filter(filter) = plan.pipeline.remove(0) else {
        panic!("expected leading filter");
    };
    let pattern = plan.pattern_plan.as_mut().expect("pattern plan");
    pattern.filters.push(filter);
    let original = pattern.join_tree.clone();
    pattern.join_tree = JoinTree::WorstCaseOptimal {
        intersection: vec![original],
        node_id_ordering: Vec::new(),
    };

    let optimized = optimize(plan, &selene_gql::OptimizeContext::default());
    let pattern = optimized.pattern_plan.as_ref().expect("pattern plan");
    assert_eq!(pattern.filters.len(), 1);
    assert!(matches!(
        pattern.join_tree,
        JoinTree::WorstCaseOptimal { .. }
    ));
}

#[test]
fn sentinel_expand_filter_pushdown_snapshot() {
    let plan = optimized_one("MATCH (a)-[e]->(b) FILTER e.weight > 1 RETURN a");
    let binding = binding_by_name(&plan, "e");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let summary = format!(
        "pattern_filters={}\nedge_filters={}",
        pattern.filters.len(),
        edge_predicate_count(&pattern.join_tree, binding),
    );
    insta::assert_snapshot!(summary, @r###"
pattern_filters=0
edge_filters=1
"###);
}
