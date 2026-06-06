//! End-to-end pattern executor tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, node_ids_for, optimized, planned, props};
use selene_core::{NodeId, Value};
use selene_testing::MockIndexCatalog;

#[test]
fn pattern_executes_simple_chain_a_to_b() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(2)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(2), Some(4)]);
}

#[test]
fn pattern_executes_simple_star_a_to_bc() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[:KNOWS]->(b), (a)-[:KNOWS]->(c) RETURN a, b, c");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);
    let mut triples = node_ids_for(&table, "a")
        .into_iter()
        .zip(node_ids_for(&table, "b"))
        .zip(node_ids_for(&table, "c"))
        .map(|((a, b), c)| (a, b, c))
        .collect::<Vec<_>>();
    triples.sort();

    assert_eq!(
        triples,
        vec![(Some(1), Some(2), Some(2)), (Some(2), Some(4), Some(4)),]
    );
}

#[test]
fn pattern_executes_simple_cycle_through_wco_marker() {
    let fixture = ExecFixture::build();
    {
        let mut txn = fixture.graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_edge(
                exec_common::db_string("KNOWS"),
                NodeId::new(4),
                NodeId::new(1),
                props([(fixture.score.clone(), Value::Int(3))]),
            )
            .expect("cycle edge inserts");
        txn.commit().expect("cycle update commits");
    }
    let plan = optimized(
        "MATCH (a)-[:KNOWS]->(b)-[:KNOWS]->(c)-[:KNOWS]->(a) RETURN a, b, c",
        &MockIndexCatalog::new(),
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert!(matches!(
        pattern.join_tree,
        selene_gql::JoinTree::WorstCaseOptimal { .. }
    ));
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);
    let mut triples = node_ids_for(&table, "a")
        .into_iter()
        .zip(node_ids_for(&table, "b"))
        .zip(node_ids_for(&table, "c"))
        .map(|((a, b), c)| (a, b, c))
        .collect::<Vec<_>>();
    triples.sort();

    assert_eq!(
        triples,
        vec![
            (Some(1), Some(2), Some(4)),
            (Some(2), Some(4), Some(1)),
            (Some(4), Some(1), Some(2)),
        ]
    );
}

#[test]
fn pattern_with_pattern_level_filters_drops_non_matching_rows() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[:KNOWS]->(b) WHERE b.score > 50 RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(2)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(4)]);
}

#[test]
fn pattern_with_index_aware_scan_inside_expand() {
    let fixture = ExecFixture::build();
    let plan = optimized(
        "MATCH (n:Person)-[:KNOWS]->(m) WHERE n.age >= 30 RETURN n, m",
        &fixture.index_catalog(),
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "n"), vec![Some(1), Some(2)]);
    assert_eq!(node_ids_for(&table, "m"), vec![Some(2), Some(4)]);
}
