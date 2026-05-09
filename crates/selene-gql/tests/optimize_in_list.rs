//! BRIEF-29 IN-list optimizer tests.

use selene_core::{IStr, intern};
use selene_gql::{
    EmptyProcedureRegistry, IndexKind, JoinTree, NodeOrEdgeScan, ScanAccess, analyze, optimize,
    parse, plan,
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
fn rewrites_small_literal_in_list_to_bitmap_union() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        istr("Person"),
        istr("email"),
        IndexKind::String,
    );
    let plan = optimized_one(
        "MATCH (p:Person) WHERE p.email IN ['alice@example.com', 'bob@example.com'] RETURN p",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(scan.access, ScanAccess::BitmapUnion { ref keys, .. } if keys.len() == 2));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn leaves_large_in_list_unchanged() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        istr("Person"),
        istr("age"),
        IndexKind::Integer,
    );
    let plan = optimized_one(
        "MATCH (p:Person) WHERE p.age IN [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17] RETURN p",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(scan.access, ScanAccess::Linear));
    assert_eq!(scan.property_predicates.len(), 1);
}

#[test]
fn sentinel_in_list_snapshot() {
    let catalog = MockIndexCatalog::new().with_node_typed_index(
        istr("Person"),
        istr("email"),
        IndexKind::String,
    );
    let plan = optimized_one(
        "MATCH (p:Person) WHERE p.email IN ['alice@example.com', 'bob@example.com'] RETURN p",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    let summary = match scan.access {
        ScanAccess::BitmapUnion { ref keys, .. } => format!(
            "access=bitmap_union\nkeys={}\nresidual_filters={}",
            keys.len(),
            scan.property_predicates.len()
        ),
        ref other => format!("unexpected={other:?}"),
    };

    insta::assert_snapshot!(summary, @r###"
access=bitmap_union
keys=2
residual_filters=0
"###);
}
