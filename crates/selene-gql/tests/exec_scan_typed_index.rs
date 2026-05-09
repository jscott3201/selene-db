//! Executor typed-index scan tests.

mod exec_common;

use exec_common::{
    ExecFixture, LARGE_COUNTER_B, execute_pattern, node_ids, optimized, planned,
    set_first_scan_access,
};
use selene_gql::{IndexHandle, IndexKind, Literal, ScanAccess, SourceSpan, TypedIndexBounds};

#[test]
fn typed_index_range_scan_uses_bounds_and_residuals() {
    let fixture = ExecFixture::build();
    let catalog = fixture.index_catalog();
    let plan = optimized(
        "MATCH (n:Person) WHERE n.age >= 30 AND n.age < 50 RETURN n",
        &catalog,
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = exec_common::first_scan(&pattern.join_tree).expect("scan");

    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            bounds: TypedIndexBounds::Range { .. },
            ..
        }
    ));
    let ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids(&table), vec![1, 2]);
}

#[test]
fn typed_index_fallback_equality_preserves_integer_precision() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH (n:Counter) RETURN n");
    let pattern = plan.pattern_plan.as_mut().expect("pattern plan");
    set_first_scan_access(
        pattern,
        ScanAccess::TypedIndexRange {
            handle: IndexHandle::new(9_001),
            property: fixture.count,
            kind: IndexKind::Integer,
            bounds: TypedIndexBounds::Equality(Literal::Integer(
                LARGE_COUNTER_B,
                SourceSpan::new(0, 1),
            )),
        },
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids(&table), vec![6]);
}
