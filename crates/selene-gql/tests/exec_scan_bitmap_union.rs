//! Executor bitmap-union scan tests.

mod exec_common;

use exec_common::{
    ExecFixture, LARGE_COUNTER_B, db_string, execute_pattern, node_ids, optimized, planned,
    set_first_scan_access,
};
use selene_gql::{IndexHandle, IndexKey, IndexKind, Literal, ScanAccess, SourceSpan};

#[test]
fn bitmap_union_scan_returns_small_in_list_matches() {
    let fixture = ExecFixture::build();
    let catalog = fixture.index_catalog();
    let plan = optimized(
        "MATCH (n:Person) WHERE n.email IN ['alice@example.com', 'cara@example.com'] RETURN n",
        &catalog,
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = exec_common::first_scan(&pattern.join_tree).expect("scan");

    assert!(matches!(
        &scan.access,
        ScanAccess::BitmapUnion { property, keys, .. }
            if *property == db_string("email") && keys.len() == 2
    ));
    let ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids(&table), vec![1, 3]);
}

#[test]
fn bitmap_union_fallback_in_list_preserves_integer_precision() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH (n:Counter) RETURN n");
    let pattern = plan.pattern_plan.as_mut().expect("pattern plan");
    set_first_scan_access(
        pattern,
        ScanAccess::BitmapUnion {
            handle: IndexHandle::new(9_002),
            property: fixture.count.clone(),
            kind: IndexKind::Integer,
            keys: vec![IndexKey::Literal(Literal::Integer(
                LARGE_COUNTER_B,
                SourceSpan::new(0, 1),
            ))],
        },
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids(&table), vec![6]);
}
