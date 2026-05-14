//! Betweenness centrality procedure adapter tests.

mod common;

use common::{
    analyze_failure, build_empty_projection, build_projection, build_unweighted_projection,
    build_weighted_projection, direct_algorithm_rows, execute_ok, execute_result, graph_with_edges,
    graph_with_labeled_weighted_edges, invalid_argument_detail, istr, registry, rows, table_values,
};
use selene_algorithms::betweenness;
use selene_algorithms_pack::{ALGO_PROCEDURE_NAMES, AlgorithmsPack};
use selene_core::{GraphId, LabelSet, NodeId, PropertyMap, Value};
use selene_gql::{AnalysisError, ExpectedType, GqlType, ProcedureRegistry, TypeMismatchContext};
use selene_graph::SharedGraph;
use selene_testing::AlgoPackCorpus;

#[test]
fn algo_betweenness_signature_matches_declared_metadata() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let metadata = lookup(&registry);

    assert_eq!(metadata.signature.parameters.len(), 2);
    assert_eq!(
        metadata.signature.parameters[0].name.as_str(),
        "projection_name"
    );
    assert_eq!(metadata.signature.parameters[0].ty, GqlType::String);
    assert!(!metadata.signature.parameters[0].nullable);
    assert_eq!(
        metadata.signature.parameters[1].name.as_str(),
        "sample_size"
    );
    assert_eq!(metadata.signature.parameters[1].ty, GqlType::Integer);
    assert!(metadata.signature.parameters[1].nullable);
    assert_eq!(
        metadata
            .output_schema
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.ty.clone()))
            .collect::<Vec<_>>(),
        vec![("node_id", GqlType::NodeRef), ("score", GqlType::Float)]
    );
}

#[test]
fn algo_betweenness_returns_node_score_rows_matching_direct_api_call_for_none_sample() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_601, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.betweenness('p', NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let expected = direct_betweenness_rows(&graph, "p", None, None);

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_betweenness_returns_node_score_rows_matching_direct_api_call_for_full_sample() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_602, &[(0, 1), (1, 2), (2, 3)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.betweenness('p', 4) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let expected = direct_betweenness_rows(&graph, "p", None, Some(4));

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_betweenness_returns_all_zero_scores_when_sample_size_zero() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_603, &[(0, 1), (1, 2)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.betweenness('p', 0) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 3);
    for row in table.rows() {
        assert_eq!(row.values()[1], Value::Float(0.0));
    }
}

#[test]
fn algo_betweenness_returns_node_score_rows_matching_direct_api_call_for_sampled_subset() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_604, &[(4, 0), (0, 1), (1, 2), (2, 3)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.betweenness('p', 4) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let expected = direct_betweenness_rows(&graph, "p", None, Some(4));

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_betweenness_sorted_desc_by_score_then_asc_by_node_id() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_edges(7_605, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.betweenness('p', NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(
        table
            .rows()
            .iter()
            .map(|row| row.values()[0].clone())
            .collect::<Vec<_>>(),
        nodes.into_iter().map(Value::NodeRef).collect::<Vec<_>>()
    );
}

#[test]
fn algo_betweenness_errors_on_missing_projection_name() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_606, &[(0, 1)]);

    let err = execute_result(
        "CALL algo.betweenness('missing', NULL) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("missing projection errors");

    assert!(invalid_argument_detail(&err).contains("projection"));
}

#[test]
fn algo_betweenness_returns_zero_rows_for_empty_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_607, &[(0, 1)]);
    build_empty_projection(&graph, &registry, "empty");

    let table = rows(execute_ok(
        "CALL algo.betweenness('empty', NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_betweenness_is_deterministic_across_repeated_calls() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_608, &[(0, 1), (0, 2), (1, 3), (2, 3)]);
    build_projection(&graph, &registry, "p");

    let first = rows(execute_ok(
        "CALL algo.betweenness('p', 2) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let second = rows(execute_ok(
        "CALL algo.betweenness('p', 2) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(table_values(&first), table_values(&second));
}

#[test]
fn algo_pack_corpus_betweenness_entry_renders_to_expected_call() {
    let rendered = AlgoPackCorpus::b4_seed().render();

    assert!(rendered.contains("betweenness_defaults [Algorithm] CALL algo.betweenness('p', NULL)"));
}

#[test]
fn algo_pack_corpus_drift_detection_pins_post_b4_procedure_count() {
    let corpus = AlgoPackCorpus::b4_seed();

    assert_eq!(corpus.entries().len(), ALGO_PROCEDURE_NAMES.len());
}

#[test]
fn algo_betweenness_ignores_projection_edge_weights() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(
        7_609,
        &["A", "B", "C"],
        &[(0, 2, 100.0), (0, 1, 1.0), (1, 2, 1.0), (2, 0, 1.0)],
    );
    build_unweighted_projection(&graph, &registry, "unweighted", &[]);
    build_weighted_projection(&graph, &registry, "weighted", &[]);

    let unweighted = rows(execute_ok(
        "CALL algo.betweenness('unweighted', NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));
    let weighted = rows(execute_ok(
        "CALL algo.betweenness('weighted', NULL) YIELD node_id, score",
        &graph,
        &registry,
    ));

    assert_eq!(table_values(&unweighted), table_values(&weighted));
}

#[test]
fn algo_betweenness_rejects_wrong_arity_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure("CALL algo.betweenness('p') YIELD node_id", &registry);

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
fn algo_betweenness_rejects_static_non_integer_sample_size_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure("CALL algo.betweenness('p', 1.5) YIELD node_id", &registry);
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

#[test]
fn value_extended_not_emitted_in_betweenness_adapter_output() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_610, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.betweenness('p', NULL) YIELD node_id, score",
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
fn nullable_option_usize_accepts_value_uint_on_all_targets_via_dynamic_arg() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let graph = graph_with_config_sample(7_611, Value::Uint(2));
    build_unweighted_projection(&graph, &registry, "p", &["Person"]);

    let table = rows(execute_ok(
        "MATCH (cfg:Config) CALL algo.betweenness('p', cfg.sample) YIELD node_id, score RETURN node_id, score",
        &graph,
        &registry,
    ));
    let expected = direct_betweenness_rows(&graph, "p", Some(&["Person"]), Some(2));

    assert_eq!(table_values(&table), expected);
}

#[test]
fn nullable_option_usize_rejects_non_integer_with_integer_or_null_detail_via_dynamic_arg() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let graph = graph_with_config_sample(7_612, Value::String(istr("bad")));
    build_unweighted_projection(&graph, &registry, "p", &["Person"]);

    let err = execute_result(
        "MATCH (cfg:Config) CALL algo.betweenness('p', cfg.sample) YIELD node_id RETURN node_id",
        &graph,
        &registry,
    )
    .expect_err("dynamic bad sample_size rejected");

    assert!(invalid_argument_detail(&err).contains("INTEGER or NULL"));
}

fn direct_betweenness_rows(
    graph: &SharedGraph,
    name: &str,
    node_labels: Option<&[&str]>,
    sample_size: Option<usize>,
) -> Vec<Vec<Value>> {
    direct_algorithm_rows(
        graph,
        name,
        node_labels.unwrap_or(&[]),
        None,
        |projection| betweenness_rows(betweenness(projection, sample_size)),
    )
}

fn betweenness_rows(result: Vec<(NodeId, f64)>) -> Vec<Vec<Value>> {
    result
        .into_iter()
        .map(|(node_id, score)| vec![Value::NodeRef(node_id), Value::Float(score)])
        .collect()
}

fn lookup(registry: &dyn ProcedureRegistry) -> selene_gql::ProcedureMetadata {
    registry
        .lookup(&[istr("algo"), istr("betweenness")])
        .expect("betweenness procedure registered")
}

fn graph_with_config_sample(id: u64, sample: Value) -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(id));
    let config = istr("Config");
    let person = istr("Person");
    let rel = istr("LINK");
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(config),
            PropertyMap::from_pairs([(istr("sample"), sample)]).expect("sample property builds"),
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
    let c = txn
        .mutator()
        .create_node(LabelSet::single(person), PropertyMap::new())
        .expect("person node inserts");
    txn.mutator()
        .create_edge(rel, a, b, PropertyMap::new())
        .expect("fixture edge inserts");
    txn.mutator()
        .create_edge(rel, b, c, PropertyMap::new())
        .expect("fixture edge inserts");
    txn.commit().expect("fixture commit succeeds");
    shared
}
