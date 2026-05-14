//! Parallelism-specific community adapter tests.

mod common;

use common::{
    analyze_failure, build_projection, execute_ok, execute_result, graph_with_edges,
    invalid_argument_detail, registry, rows, table_values,
};
use selene_algorithms_pack::AlgorithmsPack;
use selene_gql::{AnalysisError, ExpectedType, GqlType, TypeMismatchContext};

#[test]
fn algo_triangle_count_accepts_parallelism_zero_and_positive_thread_count() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_719, &[(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)]);
    build_projection(&graph, &registry, "p");

    let sequential = rows(execute_ok(
        "CALL algo.triangle_count('p', 0) YIELD node_id, triangle_count",
        &graph,
        &registry,
    ));
    let threaded = rows(execute_ok(
        "CALL algo.triangle_count('p', 4) YIELD node_id, triangle_count",
        &graph,
        &registry,
    ));
    let auto = rows(execute_ok(
        "CALL algo.triangle_count('p', NULL) YIELD node_id, triangle_count",
        &graph,
        &registry,
    ));

    assert_eq!(table_values(&sequential), table_values(&auto));
    assert_eq!(table_values(&threaded), table_values(&auto));
}

#[test]
fn algo_triangle_count_rejects_parallelism_above_adapter_cap() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_720, &[(0, 1)]);
    build_projection(&graph, &registry, "p");

    let err = execute_result(
        "CALL algo.triangle_count('p', 1025) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("oversized parallelism rejected");

    assert!(invalid_argument_detail(&err).contains("1024"));
}

#[test]
fn algo_triangle_count_rejects_wrong_arity_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure("CALL algo.triangle_count() YIELD triangle_count", &registry);

    assert!(matches!(
        err,
        AnalysisError::WrongArgumentCount {
            expected: 2,
            actual: 0,
            ..
        }
    ));
}

#[test]
fn algo_triangle_count_rejects_missing_nullable_parallelism_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure(
        "CALL algo.triangle_count('p') YIELD triangle_count",
        &registry,
    );

    assert!(matches!(
        err,
        AnalysisError::WrongArgumentCount {
            expected: 2,
            actual: 1,
            ..
        }
    ));
}

#[test]
fn algo_triangle_count_rejects_static_non_integer_parallelism_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure(
        "CALL algo.triangle_count('p', 1.5) YIELD triangle_count",
        &registry,
    );
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
        TypeMismatchContext::ProcedureArgument { position: 1, .. }
    ));
    assert_eq!(expected, ExpectedType::Specific(GqlType::Integer));
    assert_eq!(found, GqlType::Float);
}
