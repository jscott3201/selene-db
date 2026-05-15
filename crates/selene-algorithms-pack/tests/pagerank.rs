//! PageRank procedure adapter tests.

mod common;

use common::{
    analyze_failure, build_projection, build_unweighted_projection, build_weighted_projection,
    direct_pagerank_rows, execute_ok, execute_result, graph_with_edges,
    graph_with_labeled_weighted_edges, registry, rows, table_values,
};
use selene_algorithms::{PageRankConfig, Parallelism};
use selene_algorithms_pack::{
    AlgorithmsPack, DEFAULT_DAMPING, DEFAULT_MAX_ITERATIONS, DEFAULT_TOLERANCE,
};
use selene_core::Value;
use selene_gql::{AnalysisError, ExecutorError, ProcedureError};

#[test]
fn algo_pagerank_returns_node_score_rows_matching_direct_api_call() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_301, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.pagerank('p', NULL, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let actual: Vec<_> = table
        .rows()
        .iter()
        .flat_map(|row| row.values().to_vec())
        .collect();
    let expected = direct_pagerank_rows(
        &graph,
        "p",
        PageRankConfig {
            damping: DEFAULT_DAMPING,
            max_iter: DEFAULT_MAX_ITERATIONS,
            tolerance: DEFAULT_TOLERANCE,
            parallelism: Parallelism::Sequential,
        },
    );

    assert_eq!(actual, expected);
}

#[test]
fn algo_pagerank_errors_on_missing_projection_name() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_302, &[(0, 1)]);

    let err = execute_result(
        "CALL algo.pagerank('missing', NULL, NULL, NULL, NULL) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("missing projection errors");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn algo_pagerank_null_args_resolve_to_adapter_default_constants() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_303, &[(0, 1), (1, 0)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.pagerank('p', NULL, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let expected = direct_pagerank_rows(
        &graph,
        "p",
        PageRankConfig {
            damping: DEFAULT_DAMPING,
            max_iter: DEFAULT_MAX_ITERATIONS,
            tolerance: DEFAULT_TOLERANCE,
            parallelism: Parallelism::Sequential,
        },
    );

    assert_eq!(table.row_count(), expected.len() / 2);
    assert_eq!(table.rows()[0].values(), &expected[0..2]);
}

#[test]
fn algo_pagerank_accepts_zero_max_iterations() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_edges(7_304, &[(0, 1), (1, 2)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.pagerank('p', 0.85, 0, 0.000001, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 3);
    for (index, row) in table.rows().iter().enumerate() {
        assert_eq!(row.values()[0], Value::NodeRef(nodes[index]));
        let Value::Float(score) = row.values()[1] else {
            panic!("score must be float");
        };
        assert!((score - (1.0 / 3.0)).abs() < f64::EPSILON);
    }
}

#[test]
fn algo_pagerank_requires_exact_arity_five() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let one_arg = analyze_failure("CALL algo.pagerank('p') YIELD node_id", &registry);
    let two_args = analyze_failure("CALL algo.pagerank('p', 0.85) YIELD node_id", &registry);

    assert!(matches!(
        one_arg,
        AnalysisError::WrongArgumentCount {
            expected: 5,
            actual: 1,
            ..
        }
    ));
    assert!(matches!(
        two_args,
        AnalysisError::WrongArgumentCount {
            expected: 5,
            actual: 2,
            ..
        }
    ));
}

#[test]
fn algo_pagerank_two_graphs_same_name_no_collision() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph_a, _) = graph_with_edges(7_305, &[(0, 1)]);
    let (graph_b, _) = graph_with_edges(7_306, &[(0, 1), (1, 2)]);

    build_projection(&graph_a, &registry, "p");
    build_projection(&graph_b, &registry, "p");

    let rows_a = rows(execute_ok(
        "CALL algo.pagerank('p', NULL, NULL, NULL, NULL) YIELD node_id, score",
        &graph_a,
        &registry,
    ));
    let rows_b = rows(execute_ok(
        "CALL algo.pagerank('p', NULL, NULL, NULL, NULL) YIELD node_id, score",
        &graph_b,
        &registry,
    ));

    assert_eq!(rows_a.row_count(), 2);
    assert_eq!(rows_b.row_count(), 3);
}

#[test]
fn value_extended_not_emitted_in_pagerank_output() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_307, &[(0, 1)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.pagerank('p', NULL, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    for row in table.rows() {
        for value in row.values() {
            assert!(!matches!(value, Value::Extended { .. }));
        }
    }
}

#[test]
fn algo_pagerank_ignores_projection_edge_weights() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(
        7_308,
        &["Person", "Person", "Person"],
        &[(0, 1, 100.0), (1, 2, 1.0), (2, 0, 1.0)],
    );
    build_unweighted_projection(&graph, &registry, "unweighted", &[]);
    build_weighted_projection(&graph, &registry, "weighted", &[]);

    let unweighted = rows(execute_ok(
        "CALL algo.pagerank('unweighted', NULL, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let weighted = rows(execute_ok(
        "CALL algo.pagerank('weighted', NULL, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(table_values(&unweighted), table_values(&weighted));
}
