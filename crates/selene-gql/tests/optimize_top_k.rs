//! BRIEF-28 TopK optimizer tests.

use selene_gql::{
    EmptyProcedureRegistry, LimitAmount, OrderDirection, PipelineOp, analyze, optimize, parse, plan,
};

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
            PipelineOp::Project(_) => "Project",
            PipelineOp::OrderBy(_) => "OrderBy",
            PipelineOp::Limit { .. } => "Limit",
            PipelineOp::TopK { .. } => "TopK",
            other => panic!("unexpected op {other:?}"),
        })
        .collect()
}

#[test]
fn fuses_adjacent_order_by_and_limit() {
    let plan = optimized_one("MATCH (n) RETURN n ORDER BY n.age LIMIT 10");
    let Some(PipelineOp::TopK {
        keys,
        offset,
        count,
    }) = plan.pipeline.last()
    else {
        panic!("expected TopK");
    };
    assert_eq!(keys.len(), 1);
    assert!(matches!(offset, LimitAmount::Literal(0)));
    assert!(matches!(count, LimitAmount::Literal(10)));
}

#[test]
fn preserves_offset_and_sort_direction() {
    let plan = optimized_one("MATCH (n) RETURN n ORDER BY n.age DESC LIMIT 10 OFFSET 5");
    let Some(PipelineOp::TopK {
        keys,
        offset,
        count,
    }) = plan.pipeline.last()
    else {
        panic!("expected TopK");
    };
    assert_eq!(keys[0].direction, OrderDirection::Desc);
    assert!(matches!(offset, LimitAmount::Literal(5)));
    assert!(matches!(count, LimitAmount::Literal(10)));
}

#[test]
fn preserves_order_by_without_limit() {
    let plan = optimized_one("MATCH (n) RETURN n ORDER BY n.age");
    assert_eq!(variant_names(&plan), ["Project", "OrderBy"]);
}

#[test]
fn offset_only_sentinel_does_not_fuse() {
    let plan = optimized_one("MATCH (n) RETURN n ORDER BY n.age OFFSET 5");
    assert_eq!(variant_names(&plan), ["Project", "OrderBy", "Limit"]);
    let Some(PipelineOp::Limit { count, .. }) = plan.pipeline.last() else {
        panic!("expected limit");
    };
    assert!(matches!(count, LimitAmount::Literal(u64::MAX)));
}

#[test]
fn supports_multiple_sort_keys() {
    let plan = optimized_one("MATCH (n) RETURN n ORDER BY n.age, n.name LIMIT 10");
    let Some(PipelineOp::TopK { keys, .. }) = plan.pipeline.last() else {
        panic!("expected TopK");
    };
    assert_eq!(keys.len(), 2);
}

#[test]
fn sentinel_top_k_snapshot() {
    let plan = optimized_one("MATCH (n) RETURN n ORDER BY n.age DESC LIMIT 10 OFFSET 5");
    let Some(PipelineOp::TopK {
        keys,
        offset,
        count,
    }) = plan.pipeline.last()
    else {
        panic!("expected TopK");
    };
    let summary = format!(
        "op=TopK\nkeys={}\ndirection={:?}\noffset={offset:?}\ncount={count:?}",
        keys.len(),
        keys[0].direction,
    );
    insta::assert_snapshot!(summary, @r###"
op=TopK
keys=1
direction=Desc
offset=Literal(5)
count=Literal(10)
"###);
}
