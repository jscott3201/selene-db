//! Executor bitmap-union scan tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, istr, node_ids, optimized};
use selene_gql::ScanAccess;

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
        scan.access,
        ScanAccess::BitmapUnion { property, ref keys, .. }
            if property == istr("email") && keys.len() == 2
    ));
    let ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids(&table), vec![1, 3]);
}
