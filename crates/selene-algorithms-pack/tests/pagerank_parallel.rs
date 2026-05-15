//! Parallelism-specific PageRank adapter tests.

mod common;

use common::{
    build_projection, execute_ok, execute_result, graph_with_edges, invalid_argument_detail,
    registry, rows, table_values,
};
#[cfg(target_pointer_width = "32")]
use common::{build_unweighted_projection, istr};
use selene_algorithms_pack::AlgorithmsPack;
#[cfg(target_pointer_width = "32")]
use selene_core::{GraphId, LabelSet, PropertyMap, Value};
#[cfg(target_pointer_width = "32")]
use selene_graph::SharedGraph;

#[test]
fn pagerank_args_parses_parallelism_null_zero_positive() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(8_501, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let auto = rows(execute_ok(
        "CALL algo.pagerank('p', NULL, NULL, NULL, NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let sequential = rows(execute_ok(
        "CALL algo.pagerank('p', NULL, NULL, NULL, 0) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let threaded = rows(execute_ok(
        "CALL algo.pagerank('p', NULL, NULL, NULL, 4) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(table_values(&auto), table_values(&sequential));
    assert_eq!(table_values(&threaded), table_values(&sequential));
}

#[test]
fn pagerank_args_rejects_negative_parallelism() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(8_502, &[(0, 1)]);
    build_projection(&graph, &registry, "p");

    let err = execute_result(
        "CALL algo.pagerank('p', NULL, NULL, NULL, -1) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("negative parallelism rejected");

    assert!(invalid_argument_detail(&err).contains("parallelism must be NULL"));
}

#[cfg(target_pointer_width = "32")]
#[test]
fn pagerank_args_rejects_overflow_parallelism() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let graph = graph_with_config_parallelism(8_503, Value::Uint(u64::MAX));
    build_unweighted_projection(&graph, &registry, "p", &["Person"]);

    let err = execute_result(
        "MATCH (cfg:Config) CALL algo.pagerank('p', NULL, NULL, NULL, cfg.parallelism) YIELD node_id RETURN node_id",
        &graph,
        &registry,
    )
    .expect_err("overflow parallelism rejected");

    assert_eq!(
        invalid_argument_detail(&err),
        "algo.pagerank: parallelism is too large"
    );
}

#[test]
fn pagerank_args_rejects_above_1024_parallelism() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(8_504, &[(0, 1)]);
    build_projection(&graph, &registry, "p");

    let err = execute_result(
        "CALL algo.pagerank('p', NULL, NULL, NULL, 1025) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("parallelism cap rejected");

    assert!(invalid_argument_detail(&err).contains("1024"));
}

#[cfg(target_pointer_width = "32")]
fn graph_with_config_parallelism(id: u64, parallelism: Value) -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(id));
    let config = istr("Config");
    let person = istr("Person");
    let rel = istr("LINK");
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(config),
            PropertyMap::from_pairs([(istr("parallelism"), parallelism)])
                .expect("parallelism property builds"),
        )
        .expect("config node inserts");
    let a = txn
        .mutator()
        .create_node(LabelSet::single(person), PropertyMap::new())
        .expect("person node inserts");
    let b = txn
        .mutator()
        .create_node(LabelSet::single(person), PropertyMap::new())
        .expect("person node inserts");
    txn.mutator()
        .create_edge(rel, a, b, PropertyMap::new())
        .expect("fixture edge inserts");
    txn.commit().expect("fixture commit succeeds");
    shared
}
