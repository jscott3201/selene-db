//! Executor label-index scan tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, node_ids, planned, set_first_scan_access};
use selene_gql::{IndexHandle, ScanAccess};

#[test]
fn label_index_scan_uses_label_candidates() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH (n:Person) RETURN n");
    set_first_scan_access(
        plan.pattern_plan.as_mut().expect("pattern plan"),
        ScanAccess::LabelIndex {
            handle: IndexHandle::new(1),
        },
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids(&table), vec![1, 2, 3]);
}
