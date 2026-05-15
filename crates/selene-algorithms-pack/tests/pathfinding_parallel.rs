//! Parallelism-specific pathfinding adapter tests.

mod common;

use common::{
    analyze_failure, build_weighted_projection, execute_ok, execute_result,
    graph_with_labeled_weighted_edges, invalid_argument_detail, registry, rows, table_values,
};
use selene_algorithms_pack::AlgorithmsPack;
use selene_gql::{AnalysisError, ExpectedType, GqlType, TypeMismatchContext};

#[test]
fn algo_apsp_accepts_parallelism_zero_and_positive_thread_count() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(
        7_528,
        &["A", "B", "C"],
        &[(0, 1, 1.0), (1, 2, 2.0), (0, 2, 10.0)],
    );
    build_weighted_projection(&graph, &registry, "p", &[]);

    let sequential = rows(execute_ok(
        "CALL algo.apsp('p', 10, 0) YIELD source_node, target_node, cost",
        &graph,
        &registry,
    ));
    let threaded = rows(execute_ok(
        "CALL algo.apsp('p', 10, 4) YIELD source_node, target_node, cost",
        &graph,
        &registry,
    ));
    let auto = rows(execute_ok(
        "CALL algo.apsp('p', 10, NULL) YIELD source_node, target_node, cost",
        &graph,
        &registry,
    ));

    assert_eq!(table_values(&sequential), table_values(&auto));
    assert_eq!(table_values(&threaded), table_values(&auto));
}

#[test]
fn algo_apsp_rejects_parallelism_above_adapter_cap() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(7_529, &["A"], &[]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let err = execute_result(
        "CALL algo.apsp('p', 10, 1025) YIELD source_node",
        &graph,
        &registry,
    )
    .expect_err("oversized parallelism rejected");

    assert!(invalid_argument_detail(&err).contains("1024"));
}

#[test]
fn algo_apsp_rejects_negative_max_nodes_at_adapter() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(7_518, &["A"], &[]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let err = execute_result(
        "CALL algo.apsp('p', -1, NULL) YIELD source_node",
        &graph,
        &registry,
    )
    .expect_err("negative max_nodes rejected");

    assert!(invalid_argument_detail(&err).contains("non-negative"));
}

#[test]
fn algo_apsp_rejects_missing_parallelism_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure("CALL algo.apsp('p', 10) YIELD source_node", &registry);

    assert!(matches!(
        err,
        AnalysisError::WrongArgumentCount {
            expected: 3,
            actual: 2,
            ..
        }
    ));
}

#[test]
fn algo_apsp_rejects_static_non_integer_parallelism_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure("CALL algo.apsp('p', 10, 1.5) YIELD source_node", &registry);
    let AnalysisError::TypeMismatch {
        context,
        expected,
        found,
        ..
    } = err
    else {
        panic!("expected TypeMismatch, got {err:?}");
    };

    assert!(matches!(
        context,
        TypeMismatchContext::ProcedureArgument { position: 2, .. }
    ));
    assert_eq!(expected, ExpectedType::Specific(GqlType::Integer));
    assert_eq!(found, GqlType::Float);
}
