//! Executor WCO marker tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, planned};
use selene_gql::{EmptyProcedureRegistry, ExecutorError, JoinTree};

#[test]
fn wco_marker_unwraps_to_inner_join_tree_in_phase_a() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let caps = plan.impl_defined_caps;
    let ctx =
        selene_gql::TxContext::read_only(fixture.graph.read(), &caps, &EmptyProcedureRegistry);
    let expected = execute_pattern(pattern, &ctx);

    let mut wrapped = pattern.clone();
    let inner = wrapped.join_tree.clone();
    wrapped.join_tree = JoinTree::WorstCaseOptimal {
        intersection: vec![inner],
        node_id_ordering: Vec::new(),
    };

    let observed = execute_pattern(&wrapped, &ctx);

    assert_eq!(observed, expected);
}

#[test]
fn wco_with_empty_intersection_returns_implementation_defined() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_mut().expect("pattern plan");
    pattern.join_tree = JoinTree::WorstCaseOptimal {
        intersection: Vec::new(),
        node_id_ordering: Vec::new(),
    };
    let caps = plan.impl_defined_caps;
    let ctx =
        selene_gql::TxContext::read_only(fixture.graph.read(), &caps, &EmptyProcedureRegistry);

    let err = selene_gql::execute_pattern(pattern, &ctx).expect_err("wco marker errors");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "WorstCaseOptimal with empty intersection"
        }
    ));
}

#[test]
fn wco_with_multiple_branches_returns_implementation_defined() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_mut().expect("pattern plan");
    let inner = pattern.join_tree.clone();
    pattern.join_tree = JoinTree::WorstCaseOptimal {
        intersection: vec![inner.clone(), inner],
        node_id_ordering: Vec::new(),
    };
    let caps = plan.impl_defined_caps;
    let ctx =
        selene_gql::TxContext::read_only(fixture.graph.read(), &caps, &EmptyProcedureRegistry);

    let err = selene_gql::execute_pattern(pattern, &ctx).expect_err("wco marker errors");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "WorstCaseOptimal with multiple intersections"
        }
    ));
}
