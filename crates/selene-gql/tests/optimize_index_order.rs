//! BRIEF-29 index-order optimizer tests.

use selene_core::{IStr, intern};
use selene_gql::{
    EmptyProcedureRegistry, IndexKind, OrderAccess, PipelineOp, analyze, optimize, parse, plan,
};
use selene_testing::MockIndexCatalog;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn optimized_one(source: &str, catalog: &MockIndexCatalog) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    let ctx = selene_gql::OptimizeContext::default().with_index_catalog(catalog);
    optimize(plan, &ctx)
}

#[test]
fn annotates_order_key_with_typed_index_access() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        istr("Person"),
        istr("age"),
        IndexKind::Integer,
    );
    let plan = optimized_one("MATCH (n:Person) RETURN n ORDER BY n.age", &catalog);

    let order_key = plan
        .pipeline
        .iter()
        .find_map(|op| match op {
            PipelineOp::OrderBy(keys) => keys.first(),
            _ => None,
        })
        .expect("order key");
    assert!(matches!(
        order_key.access,
        Some(OrderAccess::TypedIndex { .. })
    ));
}

#[test]
fn topk_preserves_order_access_after_fusion() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        istr("Person"),
        istr("age"),
        IndexKind::Integer,
    );
    let plan = optimized_one(
        "MATCH (n:Person) RETURN n ORDER BY n.age LIMIT 10",
        &catalog,
    );

    let order_key = plan
        .pipeline
        .iter()
        .find_map(|op| match op {
            PipelineOp::TopK { keys, .. } => keys.first(),
            _ => None,
        })
        .expect("topk order key");
    assert!(matches!(
        order_key.access,
        Some(OrderAccess::TypedIndex { .. })
    ));
}

#[test]
fn sentinel_index_order_snapshot() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        istr("Person"),
        istr("age"),
        IndexKind::Integer,
    );
    let plan = optimized_one(
        "MATCH (n:Person) RETURN n ORDER BY n.age LIMIT 10",
        &catalog,
    );
    let summary = plan
        .pipeline
        .iter()
        .find_map(|op| match op {
            PipelineOp::TopK { keys, .. } => Some(format!(
                "op=topk\nkeys={}\naccess={}",
                keys.len(),
                usize::from(matches!(
                    keys.first().and_then(|key| key.access.as_ref()),
                    Some(OrderAccess::TypedIndex { .. })
                ))
            )),
            _ => None,
        })
        .expect("topk summary");

    insta::assert_snapshot!(summary, @r###"
op=topk
keys=1
access=1
"###);
}
