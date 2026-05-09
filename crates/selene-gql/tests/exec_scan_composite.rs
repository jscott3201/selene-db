//! Executor composite-lookup scan tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, node_ids, optimized};
use selene_gql::ScanAccess;

#[test]
fn composite_lookup_scan_filters_by_all_keys() {
    let fixture = ExecFixture::build();
    let catalog = fixture.index_catalog();
    let plan = optimized(
        "MATCH (n:Person) WHERE n.kind = 'person' AND n.tenant = 't1' RETURN n",
        &catalog,
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = exec_common::first_scan(&pattern.join_tree).expect("scan");

    assert!(matches!(scan.access, ScanAccess::CompositeLookup { .. }));
    let ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids(&table), vec![1, 2]);
}
