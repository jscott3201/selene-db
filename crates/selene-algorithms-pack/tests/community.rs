//! Community procedure adapter tests.

mod common;

use common::{
    analyze_failure, assert_no_extended, build_empty_projection, build_projection,
    build_unweighted_projection, build_weighted_projection, direct_algorithm_rows, execute_ok,
    execute_result, graph_with_count_and_weighted_edges, graph_with_edges,
    graph_with_labeled_weighted_edges, invalid_argument_detail, istr, registry, rows, table_values,
};
use selene_algorithms::{
    Parallelism, TriangleCountConfig, label_propagation, louvain, triangle_count,
};
use selene_algorithms_pack::{ALGO_PROCEDURE_NAMES, AlgorithmsPack};
use selene_core::{NodeId, Value};
use selene_gql::{AnalysisError, ExpectedType, GqlType, ProcedureRegistry, TypeMismatchContext};
use selene_graph::SharedGraph;
use selene_testing::AlgoPackCorpus;

const DEFAULT_MAX_ITER_LABEL_PROPAGATION: usize = 50;
const DEFAULT_MAX_ITER_LOUVAIN: usize = 50;

#[test]
fn algo_label_propagation_signature_matches_declared_metadata() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let metadata = lookup(&registry, &["algo", "label_propagation"]);

    assert_eq!(metadata.signature.parameters.len(), 2);
    assert_eq!(
        metadata.signature.parameters[0].name.as_str(),
        "projection_name"
    );
    assert_eq!(metadata.signature.parameters[0].ty, GqlType::String);
    assert!(!metadata.signature.parameters[0].nullable);
    assert_eq!(metadata.signature.parameters[1].name.as_str(), "max_iter");
    assert_eq!(metadata.signature.parameters[1].ty, GqlType::Integer);
    assert!(metadata.signature.parameters[1].nullable);
    assert_eq!(
        output_columns(&metadata),
        vec![
            ("node_id", GqlType::NodeRef),
            ("community", GqlType::NodeRef)
        ]
    );
}

#[test]
fn algo_louvain_signature_matches_declared_metadata() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let metadata = lookup(&registry, &["algo", "louvain"]);

    assert_eq!(metadata.signature.parameters.len(), 2);
    assert_eq!(
        metadata.signature.parameters[0].name.as_str(),
        "projection_name"
    );
    assert_eq!(metadata.signature.parameters[0].ty, GqlType::String);
    assert!(!metadata.signature.parameters[0].nullable);
    assert_eq!(metadata.signature.parameters[1].name.as_str(), "max_iter");
    assert_eq!(metadata.signature.parameters[1].ty, GqlType::Integer);
    assert!(metadata.signature.parameters[1].nullable);
    assert_eq!(
        output_columns(&metadata),
        vec![
            ("node_id", GqlType::NodeRef),
            ("community", GqlType::NodeRef),
            ("level", GqlType::Uint64),
        ]
    );
}

#[test]
fn algo_triangle_count_signature_matches_declared_metadata() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let metadata = lookup(&registry, &["algo", "triangle_count"]);

    assert_eq!(metadata.signature.parameters.len(), 2);
    assert_eq!(
        metadata.signature.parameters[0].name.as_str(),
        "projection_name"
    );
    assert_eq!(metadata.signature.parameters[0].ty, GqlType::String);
    assert!(!metadata.signature.parameters[0].nullable);
    assert_eq!(
        metadata.signature.parameters[1].name.as_str(),
        "parallelism"
    );
    assert_eq!(metadata.signature.parameters[1].ty, GqlType::Integer);
    assert!(metadata.signature.parameters[1].nullable);
    assert_eq!(
        output_columns(&metadata),
        vec![
            ("node_id", GqlType::NodeRef),
            ("triangle_count", GqlType::Uint64),
        ]
    );
}

#[test]
fn algo_label_propagation_matches_direct_api_for_null_max_iter() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_701, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.label_propagation('p', NULL) YIELD node_id, community",
        &graph,
        &registry,
    ));
    let expected =
        direct_label_propagation_rows(&graph, "p", None, DEFAULT_MAX_ITER_LABEL_PROPAGATION);

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_label_propagation_matches_direct_api_for_zero_max_iter() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_702, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.label_propagation('p', 0) YIELD node_id, community",
        &graph,
        &registry,
    ));
    let expected = direct_label_propagation_rows(&graph, "p", None, 0);

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_louvain_matches_direct_api_for_null_max_iter() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(
        7_703,
        &[
            (0, 1),
            (1, 0),
            (1, 2),
            (2, 1),
            (0, 2),
            (2, 0),
            (3, 4),
            (4, 3),
            (4, 5),
            (5, 4),
            (3, 5),
            (5, 3),
        ],
    );
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.louvain('p', NULL) YIELD node_id, community, level",
        &graph,
        &registry,
    ));
    let expected = direct_louvain_rows(&graph, "p", None, DEFAULT_MAX_ITER_LOUVAIN);

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_triangle_count_matches_direct_api() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_704, &[(0, 1), (1, 2), (2, 0), (0, 3)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.triangle_count('p', NULL) YIELD node_id, triangle_count",
        &graph,
        &registry,
    ));
    let expected = direct_triangle_count_rows(&graph, "p", None);

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_label_propagation_rows_sorted_asc_by_node_id() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_edges(7_705, &[(2, 3), (0, 1), (1, 2)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.label_propagation('p', NULL) YIELD node_id, community",
        &graph,
        &registry,
    ));

    assert_eq!(
        node_column(&table, 0),
        nodes.into_iter().map(Value::NodeRef).collect::<Vec<_>>()
    );
}

#[test]
fn algo_triangle_count_rows_sorted_desc_by_count_then_asc_by_node_id() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_count_and_weighted_edges(
        7_706,
        5,
        &[
            (0, 1, 1.0),
            (1, 2, 1.0),
            (2, 0, 1.0),
            (0, 3, 1.0),
            (1, 3, 1.0),
            (2, 3, 1.0),
        ],
    );
    build_unweighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "CALL algo.triangle_count('p', NULL) YIELD node_id, triangle_count",
        &graph,
        &registry,
    ));

    assert_eq!(
        table_values(&table),
        vec![
            vec![Value::NodeRef(nodes[0]), Value::Uint(3)],
            vec![Value::NodeRef(nodes[1]), Value::Uint(3)],
            vec![Value::NodeRef(nodes[2]), Value::Uint(3)],
            vec![Value::NodeRef(nodes[3]), Value::Uint(3)],
            vec![Value::NodeRef(nodes[4]), Value::Uint(0)],
        ]
    );
}

#[test]
fn algo_label_propagation_returns_zero_rows_for_empty_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_707, &[(0, 1)]);
    build_empty_projection(&graph, &registry, "empty");

    let table = rows(execute_ok(
        "CALL algo.label_propagation('empty', NULL) YIELD node_id, community",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_louvain_returns_zero_rows_for_empty_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_708, &[(0, 1)]);
    build_empty_projection(&graph, &registry, "empty");

    let table = rows(execute_ok(
        "CALL algo.louvain('empty', NULL) YIELD node_id, community, level",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_triangle_count_returns_zero_rows_for_empty_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_709, &[(0, 1)]);
    build_empty_projection(&graph, &registry, "empty");

    let table = rows(execute_ok(
        "CALL algo.triangle_count('empty', NULL) YIELD node_id, triangle_count",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_label_propagation_errors_on_missing_projection_name() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_710, &[(0, 1)]);

    let err = execute_result(
        "CALL algo.label_propagation('missing', NULL) YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("missing projection errors");

    assert!(invalid_argument_detail(&err).contains("projection"));
}

#[test]
fn algo_label_propagation_ignores_projection_edge_weights() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(
        7_711,
        &["A", "B", "C"],
        &[(0, 2, 100.0), (0, 1, 1.0), (1, 2, 1.0), (2, 0, 1.0)],
    );
    build_unweighted_projection(&graph, &registry, "unweighted", &[]);
    build_weighted_projection(&graph, &registry, "weighted", &[]);

    let unweighted = rows(execute_ok(
        "CALL algo.label_propagation('unweighted', NULL) YIELD node_id, community",
        &graph,
        &registry,
    ));
    let weighted = rows(execute_ok(
        "CALL algo.label_propagation('weighted', NULL) YIELD node_id, community",
        &graph,
        &registry,
    ));

    assert_eq!(table_values(&unweighted), table_values(&weighted));
}

#[test]
fn algo_louvain_differs_across_unweighted_and_weighted_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = two_cliques_with_heavy_bridge(7_712);
    build_unweighted_projection(&graph, &registry, "unweighted", &[]);
    build_weighted_projection(&graph, &registry, "weighted", &[]);

    let unweighted = rows(execute_ok(
        "CALL algo.louvain('unweighted', NULL) YIELD node_id, community",
        &graph,
        &registry,
    ));
    let weighted = rows(execute_ok(
        "CALL algo.louvain('weighted', NULL) YIELD node_id, community",
        &graph,
        &registry,
    ));

    assert!(!co_member(&unweighted, nodes[2], nodes[3]));
    assert!(co_member(&weighted, nodes[2], nodes[3]));
}

#[test]
fn algo_triangle_count_ignores_projection_edge_weights() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(
        7_713,
        &["A", "B", "C", "D"],
        &[(0, 1, 100.0), (1, 2, 2.0), (2, 0, 1.0), (0, 3, 50.0)],
    );
    build_unweighted_projection(&graph, &registry, "unweighted", &[]);
    build_weighted_projection(&graph, &registry, "weighted", &[]);

    let unweighted = rows(execute_ok(
        "CALL algo.triangle_count('unweighted', NULL) YIELD node_id, triangle_count",
        &graph,
        &registry,
    ));
    let weighted = rows(execute_ok(
        "CALL algo.triangle_count('weighted', NULL) YIELD node_id, triangle_count",
        &graph,
        &registry,
    ));

    assert_eq!(table_values(&unweighted), table_values(&weighted));
}

#[test]
fn algo_label_propagation_community_id_round_trips_via_noderef_match() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_714, &[(0, 1), (1, 2), (2, 0), (3, 4)]);
    build_projection(&graph, &registry, "p");

    let adapter_rows = rows(execute_ok(
        "CALL algo.label_propagation('p', NULL) YIELD node_id, community",
        &graph,
        &registry,
    ));
    let round_trip = rows(execute_ok(
        "MATCH (n) CALL algo.label_propagation('p', NULL) YIELD node_id, community WITH node_id, community, n WHERE n = community RETURN node_id, community, n",
        &graph,
        &registry,
    ));

    assert_eq!(round_trip.row_count(), adapter_rows.row_count());
    for row in round_trip.rows() {
        assert_eq!(row.values()[1], row.values()[2]);
    }
}

#[test]
fn algo_louvain_output_value_types_match_declared_columns() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_715, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    assert_eq!(
        output_types(&registry, &["algo", "louvain"]),
        vec![GqlType::NodeRef, GqlType::NodeRef, GqlType::Uint64]
    );
    let table = rows(execute_ok(
        "CALL algo.louvain('p', NULL) YIELD node_id, community, level",
        &graph,
        &registry,
    ));

    for row in table.rows() {
        assert!(matches!(row.values()[0], Value::NodeRef(_)));
        assert!(matches!(row.values()[1], Value::NodeRef(_)));
        assert!(matches!(row.values()[2], Value::Uint(_)));
    }
}

#[test]
fn algo_louvain_is_deterministic_across_repeated_calls() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = two_cliques_with_heavy_bridge(7_716);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let first = rows(execute_ok(
        "CALL algo.louvain('p', NULL) YIELD node_id, community, level",
        &graph,
        &registry,
    ));
    let second = rows(execute_ok(
        "CALL algo.louvain('p', NULL) YIELD node_id, community, level",
        &graph,
        &registry,
    ));

    assert_eq!(table_values(&first), table_values(&second));
}

#[test]
fn algo_label_propagation_rejects_wrong_arity_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure(
        "CALL algo.label_propagation('p') YIELD community",
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
fn algo_louvain_rejects_static_non_integer_max_iter_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure("CALL algo.louvain('p', 1.5) YIELD community", &registry);
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
fn algo_louvain_emits_level_zero_for_all_rows_in_v1_0() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_717, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.louvain('p', NULL) YIELD level",
        &graph,
        &registry,
    ));

    assert!(
        table
            .rows()
            .iter()
            .all(|row| row.values() == [Value::Uint(0)])
    );
}

#[test]
fn value_extended_not_emitted_in_any_community_adapter_output() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = two_cliques_with_heavy_bridge(7_718);
    build_weighted_projection(&graph, &registry, "p", &[]);

    for source in [
        "CALL algo.label_propagation('p', NULL) YIELD node_id, community",
        "CALL algo.louvain('p', NULL) YIELD node_id, community, level",
        "CALL algo.triangle_count('p', NULL) YIELD node_id, triangle_count",
    ] {
        let table = rows(execute_ok(source, &graph, &registry));
        for row in table.rows() {
            for value in row.values() {
                assert_no_extended(value);
            }
        }
    }
}

#[test]
fn algo_pack_corpus_community_entries_render_to_expected_calls() {
    let rendered = AlgoPackCorpus::b5_seed().render();

    assert!(
        rendered.contains(
            "label_propagation_defaults [Algorithm] CALL algo.label_propagation('p', NULL)"
        )
    );
    assert!(rendered.contains("louvain_defaults [Algorithm] CALL algo.louvain('p', NULL)"));
    assert!(rendered.contains("triangle_count [Algorithm] CALL algo.triangle_count('p', NULL)"));
}

#[test]
fn algo_pack_corpus_drift_detection_pins_post_b5_procedure_count() {
    let corpus = AlgoPackCorpus::b5_seed();

    assert_eq!(corpus.entries().len(), ALGO_PROCEDURE_NAMES.len());
}

fn direct_label_propagation_rows(
    graph: &SharedGraph,
    name: &str,
    weight_property: Option<&str>,
    max_iter: usize,
) -> Vec<Vec<Value>> {
    direct_algorithm_rows(graph, name, &[], weight_property, |projection| {
        label_propagation(projection, max_iter)
            .into_iter()
            .map(node_community_row)
            .collect()
    })
}

fn direct_louvain_rows(
    graph: &SharedGraph,
    name: &str,
    weight_property: Option<&str>,
    max_iter: usize,
) -> Vec<Vec<Value>> {
    direct_algorithm_rows(graph, name, &[], weight_property, |projection| {
        louvain(projection, max_iter)
            .into_iter()
            .map(louvain_row)
            .collect()
    })
}

fn direct_triangle_count_rows(
    graph: &SharedGraph,
    name: &str,
    weight_property: Option<&str>,
) -> Vec<Vec<Value>> {
    direct_algorithm_rows(graph, name, &[], weight_property, |projection| {
        triangle_count(
            projection,
            TriangleCountConfig {
                parallelism: Parallelism::Auto,
            },
        )
        .into_iter()
        .map(|(node_id, count)| vec![Value::NodeRef(node_id), Value::Uint(count as u64)])
        .collect()
    })
}

fn node_community_row((node_id, community_id): (NodeId, u64)) -> Vec<Value> {
    vec![
        Value::NodeRef(node_id),
        Value::NodeRef(NodeId::new(community_id)),
    ]
}

fn louvain_row((node_id, community_id, level): (NodeId, u64, u32)) -> Vec<Value> {
    vec![
        Value::NodeRef(node_id),
        Value::NodeRef(NodeId::new(community_id)),
        Value::Uint(u64::from(level)),
    ]
}

fn lookup(registry: &dyn ProcedureRegistry, name: &[&str]) -> selene_gql::ProcedureMetadata {
    let name: Vec<_> = name.iter().map(|segment| istr(segment)).collect();
    registry.lookup(&name).expect("procedure registered")
}

fn output_columns(metadata: &selene_gql::ProcedureMetadata) -> Vec<(&str, GqlType)> {
    metadata
        .output_schema
        .columns
        .iter()
        .map(|column| (column.name.as_str(), column.ty.clone()))
        .collect()
}

fn output_types(registry: &dyn ProcedureRegistry, name: &[&str]) -> Vec<GqlType> {
    lookup(registry, name)
        .output_schema
        .columns
        .into_iter()
        .map(|column| column.ty)
        .collect()
}

fn node_column(table: &selene_gql::BindingTable, index: usize) -> Vec<Value> {
    table
        .rows()
        .iter()
        .map(|row| row.values()[index].clone())
        .collect()
}

fn co_member(table: &selene_gql::BindingTable, left: NodeId, right: NodeId) -> bool {
    community_for(table, left) == community_for(table, right)
}

fn community_for(table: &selene_gql::BindingTable, node: NodeId) -> NodeId {
    table
        .rows()
        .iter()
        .find_map(|row| match row.values() {
            [Value::NodeRef(node_id), Value::NodeRef(community), ..] if *node_id == node => {
                Some(*community)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing community row for {node:?}"))
}

fn two_cliques_with_heavy_bridge(id: u64) -> (SharedGraph, Vec<NodeId>) {
    graph_with_count_and_weighted_edges(
        id,
        6,
        &[
            (0, 1, 1.0),
            (1, 0, 1.0),
            (1, 2, 1.0),
            (2, 1, 1.0),
            (0, 2, 1.0),
            (2, 0, 1.0),
            (3, 4, 1.0),
            (4, 3, 1.0),
            (4, 5, 1.0),
            (5, 4, 1.0),
            (3, 5, 1.0),
            (5, 3, 1.0),
            (2, 3, 100.0),
            (3, 2, 100.0),
        ],
    )
}
