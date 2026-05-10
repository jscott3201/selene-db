//! Executor linear scan tests.

mod exec_common;

use exec_common::{ExecFixture, edge_ids, execute_pattern, node_ids, planned};
use selene_gql::{JoinTree, NodeOrEdgeScan, ScanKind};

#[test]
fn linear_node_scan_applies_pattern_filters() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n) WHERE n.score > 5 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids(&table), vec![1, 3, 4]);
}

#[test]
fn linear_edge_scan_returns_edge_refs() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH ()-[e:KNOWS]->() RETURN e");
    let pattern = plan.pattern_plan.as_mut().expect("pattern plan");
    let edge_scan = match &pattern.join_tree {
        JoinTree::Expand { edge, .. } => NodeOrEdgeScan {
            binding: edge.binding,
            hidden_binding: edge.hidden_binding,
            kind: ScanKind::Edge,
            label_predicate: edge.label_predicate.clone(),
            property_predicates: edge.property_predicates.clone(),
            access: edge.access.clone(),
            span: edge.span,
        },
        other => panic!("expected expand tree, got {other:?}"),
    };
    pattern.join_tree = JoinTree::Scan(edge_scan);
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(edge_ids(&table), vec![1, 2]);
}

#[test]
fn consumed_predicates_are_not_re_evaluated() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH (n:Person) WHERE false RETURN n");
    plan.pattern_plan.as_mut().expect("pattern plan").filters[0].index_consumed = true;
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids(&table), vec![1, 2, 3]);
}
