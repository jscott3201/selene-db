//! Executor typed-index scan tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, node_ids, optimized};
use selene_gql::{ScanAccess, TypedIndexBounds};

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
