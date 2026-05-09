//! BRIEF-29 range-index optimizer tests.

use selene_core::{IStr, intern};
use selene_gql::{
    EmptyProcedureRegistry, IndexKind, JoinTree, NodeOrEdgeScan, ScanAccess, TypedIndexBounds,
    analyze, optimize, parse, plan,
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

fn person_catalog() -> MockIndexCatalog {
    MockIndexCatalog::new()
        .with_node_typed_index(istr("Person"), istr("age"), IndexKind::Integer)
        .with_node_typed_index(istr("Person"), istr("name"), IndexKind::String)
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
fn rewrites_greater_than_to_typed_index_range() {
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age > 30 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            bounds: TypedIndexBounds::GreaterThan(_),
            ..
        }
    ));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn combines_lower_and_upper_range_bounds() {
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age >= 30 AND n.age < 60 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            bounds: TypedIndexBounds::Range {
                lo_inclusive: true,
                hi_inclusive: false,
                ..
            },
            ..
        }
    ));
}

#[test]
fn leaves_type_mismatch_unchanged() {
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age = 'old' RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(scan.access, ScanAccess::Linear));
    assert_eq!(scan.property_predicates.len(), 1);
}

#[test]
fn sentinel_range_index_scan_snapshot() {
    let plan = optimized_one(
        "MATCH (n:Person) WHERE n.age >= 30 AND n.age < 60 RETURN n",
        &person_catalog(),
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    let summary = match scan.access {
        ScanAccess::TypedIndexRange {
            bounds:
                TypedIndexBounds::Range {
                    lo_inclusive,
                    hi_inclusive,
                    ..
                },
            ..
        } => format!(
            "access=range\nlo_inclusive={lo_inclusive}\nhi_inclusive={hi_inclusive}\nresidual_filters={}",
            scan.property_predicates.len()
        ),
        ref other => format!("unexpected={other:?}"),
    };

    insta::assert_snapshot!(summary, @r###"
access=range
lo_inclusive=true
hi_inclusive=false
residual_filters=0
"###);
}
