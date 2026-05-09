//! BRIEF-28 filter-pushdown optimizer tests.

use selene_gql::{EmptyProcedureRegistry, PipelineOp, analyze, optimize, parse, plan};

fn optimized_one(source: &str) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    optimize(plan, &selene_gql::OptimizeContext::default())
}

fn variant_names(plan: &selene_gql::ExecutionPlan) -> Vec<&'static str> {
    plan.pipeline
        .iter()
        .map(|op| match op {
            PipelineOp::Filter(_) => "Filter",
            PipelineOp::Project(_) => "Project",
            PipelineOp::Let(_) => "Let",
            PipelineOp::Limit { .. } => "Limit",
            PipelineOp::TopK { .. } => "TopK",
            other => panic!("unexpected op {other:?}"),
        })
        .collect()
}

#[test]
fn pushes_leading_filter_into_pattern() {
    let plan = optimized_one("MATCH (n) FILTER n.a > 1 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert_eq!(pattern.filters.len(), 1);
    assert_eq!(variant_names(&plan), ["Project"]);
}

#[test]
fn does_not_push_filter_after_let_boundary() {
    let plan = optimized_one("MATCH (n) LET x = n.a + 1 FILTER x > 0 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert!(pattern.filters.is_empty());
    assert_eq!(variant_names(&plan), ["Let", "Filter", "Project"]);
}

#[test]
fn stops_at_first_non_filter_boundary() {
    let plan = optimized_one("MATCH (n) FILTER n.a > 1 LIMIT 10 FILTER n.b < 5 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert_eq!(pattern.filters.len(), 1);
    assert_eq!(variant_names(&plan), ["Limit", "Filter", "Project"]);
}

#[test]
fn no_pattern_plan_is_unchanged() {
    let plan = optimized_one("RETURN 1 + 2 AS x");
    assert!(plan.pattern_plan.is_none());
    assert_eq!(variant_names(&plan), ["Project"]);
}

#[test]
fn sentinel_filter_pushdown_snapshot() {
    let plan = optimized_one("MATCH (n) FILTER n.a > 1 LIMIT 10 FILTER n.b < 5 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let summary = format!(
        "pattern_filters={}\npipeline={}",
        pattern.filters.len(),
        variant_names(&plan).join(","),
    );
    insta::assert_snapshot!(summary, @r###"
pattern_filters=1
pipeline=Limit,Filter,Project
"###);
}
