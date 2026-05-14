//! Pathfinding procedure adapter tests.

mod common;

use common::{
    analyze_failure, assert_invalid_argument, build_empty_projection, build_unweighted_projection,
    build_weighted_projection, direct_algorithm_rows, execute_ok, execute_result,
    graph_with_labeled_unweighted_edges, graph_with_labeled_weighted_edges,
    invalid_argument_detail, istr, registry, rows, table_values,
};
use selene_algorithms::{apsp, dijkstra, sssp};
use selene_algorithms_pack::AlgorithmsPack;
use selene_core::{NodeId, Value};
use selene_gql::{
    AnalysisError, BindingTable, ExpectedType, GqlType, ProcedureRegistry, TypeMismatchContext,
};

#[test]
fn algo_dijkstra_returns_single_row_with_cost_path_length_matching_direct_api_call() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_labeled_weighted_edges(
        7_501,
        &["Source", "Mid", "Target"],
        &[(0, 1, 1.5), (1, 2, 2.5), (0, 2, 10.0)],
    );
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD cost, path, length RETURN cost, path, length",
        &graph,
        &registry,
    ));
    let expected = direct_algorithm_rows(&graph, "p", &[], Some("w"), |projection| {
        dijkstra_rows(dijkstra(projection, nodes[0], nodes[2]).expect("direct dijkstra succeeds"))
    });

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_dijkstra_path_column_is_value_list_of_node_ref() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) =
        graph_with_labeled_weighted_edges(7_502, &["Source", "Target"], &[(0, 1, 3.0)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    assert_eq!(
        output_types(&registry, &["algo", "dijkstra"]),
        vec![
            GqlType::Float,
            GqlType::List(Box::new(GqlType::NodeRef)),
            GqlType::Uint64,
        ]
    );
    let table = rows(execute_ok(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD path RETURN path",
        &graph,
        &registry,
    ));

    assert_eq!(
        table.rows()[0].values(),
        &[Value::List(vec![
            Value::NodeRef(nodes[0]),
            Value::NodeRef(nodes[1])
        ])]
    );
}

#[test]
fn algo_dijkstra_errors_on_missing_projection_name() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) =
        graph_with_labeled_weighted_edges(7_503, &["Source", "Target"], &[(0, 1, 1.0)]);

    assert_invalid_argument(
        execute_result(
            "MATCH (a:Source), (b:Target) CALL algo.dijkstra('missing', a, b) YIELD cost RETURN cost",
            &graph,
            &registry,
        )
        .expect_err("missing projection errors"),
    );
}

#[test]
fn algo_dijkstra_returns_zero_rows_when_no_path_exists() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) =
        graph_with_labeled_weighted_edges(7_504, &["Source", "Mid", "Target"], &[(0, 1, 1.0)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD cost RETURN cost",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_dijkstra_returns_zero_rows_when_source_not_in_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) =
        graph_with_labeled_weighted_edges(7_505, &["Outside", "Inside"], &[(0, 1, 1.0)]);
    build_weighted_projection(&graph, &registry, "p", &["Inside"]);

    let table = rows(execute_ok(
        "MATCH (a:Outside), (b:Inside) CALL algo.dijkstra('p', a, b) YIELD cost RETURN cost",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_dijkstra_returns_zero_rows_when_target_not_in_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) =
        graph_with_labeled_weighted_edges(7_506, &["Inside", "Outside"], &[(0, 1, 1.0)]);
    build_weighted_projection(&graph, &registry, "p", &["Inside"]);

    let table = rows(execute_ok(
        "MATCH (a:Inside), (b:Outside) CALL algo.dijkstra('p', a, b) YIELD cost RETURN cost",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_dijkstra_zero_step_path_when_from_equals_to() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_labeled_weighted_edges(7_507, &["Source"], &[]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source) CALL algo.dijkstra('p', a, a) YIELD cost, path, length RETURN cost, path, length",
        &graph,
        &registry,
    ));

    assert_eq!(
        table.rows()[0].values(),
        &[
            Value::Float(0.0),
            Value::List(vec![Value::NodeRef(nodes[0])]),
            Value::Uint(1),
        ]
    );
}

#[test]
fn algo_dijkstra_tie_break_smaller_nodeid_predecessor_wins() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_labeled_weighted_edges(
        7_508,
        &["Source", "SmallPred", "LargePred", "Target"],
        &[(0, 1, 4.0), (1, 3, 1.0), (0, 2, 1.0), (2, 3, 4.0)],
    );
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD path RETURN path",
        &graph,
        &registry,
    ));

    assert_eq!(
        table.rows()[0].values(),
        &[Value::List(vec![
            Value::NodeRef(nodes[0]),
            Value::NodeRef(nodes[1]),
            Value::NodeRef(nodes[3]),
        ])]
    );
}

#[test]
fn algo_sssp_returns_node_cost_rows_matching_direct_api_call() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_labeled_weighted_edges(
        7_509,
        &["Source", "Mid", "Target"],
        &[(0, 1, 1.0), (1, 2, 2.0), (0, 2, 10.0)],
    );
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source) CALL algo.sssp('p', a) YIELD target_node, cost RETURN target_node, cost",
        &graph,
        &registry,
    ));
    let expected = direct_algorithm_rows(&graph, "p", &[], Some("w"), |projection| {
        sssp_rows(sssp(projection, nodes[0]).expect("direct sssp succeeds"))
    });

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_sssp_includes_source_with_zero_cost() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) =
        graph_with_labeled_weighted_edges(7_510, &["Source", "Target"], &[(0, 1, 1.0)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source) CALL algo.sssp('p', a) YIELD target_node, cost RETURN target_node, cost",
        &graph,
        &registry,
    ));

    assert_eq!(
        table.rows()[0].values(),
        &[Value::NodeRef(nodes[0]), Value::Float(0.0)]
    );
}

#[test]
fn algo_sssp_excludes_unreachable_nodes() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) =
        graph_with_labeled_weighted_edges(7_511, &["Source", "Target", "Other"], &[(0, 1, 1.0)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source) CALL algo.sssp('p', a) YIELD target_node RETURN target_node",
        &graph,
        &registry,
    ));
    let emitted: Vec<_> = table
        .rows()
        .iter()
        .map(|row| row.values()[0].clone())
        .collect();

    assert!(!emitted.contains(&Value::NodeRef(nodes[2])));
}

#[test]
fn algo_sssp_returns_zero_rows_when_source_not_in_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) =
        graph_with_labeled_weighted_edges(7_512, &["Outside", "Inside"], &[(0, 1, 1.0)]);
    build_weighted_projection(&graph, &registry, "p", &["Inside"]);

    let table = rows(execute_ok(
        "MATCH (a:Outside) CALL algo.sssp('p', a) YIELD target_node RETURN target_node",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_sssp_sorted_asc_by_node_id() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(
        7_513,
        &["Source", "B", "C", "D"],
        &[(0, 3, 1.0), (0, 2, 1.0), (0, 1, 1.0)],
    );
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source) CALL algo.sssp('p', a) YIELD target_node, cost RETURN target_node, cost",
        &graph,
        &registry,
    ));
    let ids = node_ids_in_column(&table, 0);

    assert!(
        ids.windows(2)
            .all(|window| window[0].get() < window[1].get())
    );
}

#[test]
fn algo_apsp_returns_source_target_cost_rows_matching_direct_api_call() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(
        7_514,
        &["A", "B", "C"],
        &[(0, 1, 1.0), (1, 2, 2.0), (0, 2, 10.0)],
    );
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "CALL algo.apsp('p', 10) YIELD source_node, target_node, cost",
        &graph,
        &registry,
    ));
    let expected = direct_algorithm_rows(&graph, "p", &[], Some("w"), |projection| {
        apsp_rows(apsp(projection, 10).expect("direct apsp succeeds"))
    });

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_apsp_excludes_self_pairs() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) =
        graph_with_labeled_weighted_edges(7_515, &["A", "B"], &[(0, 1, 1.0), (1, 0, 1.0)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "CALL algo.apsp('p', 10) YIELD source_node, target_node, cost",
        &graph,
        &registry,
    ));

    for row in table.rows() {
        assert_ne!(row.values()[0], row.values()[1]);
    }
}

#[test]
fn algo_apsp_excludes_unreachable_pairs() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_labeled_weighted_edges(7_516, &["A", "B", "C"], &[(0, 1, 1.0)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "CALL algo.apsp('p', 10) YIELD source_node, target_node, cost",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 1);
    assert_eq!(
        table.rows()[0].values(),
        &[
            Value::NodeRef(nodes[0]),
            Value::NodeRef(nodes[1]),
            Value::Float(1.0),
        ]
    );

    build_empty_projection(&graph, &registry, "empty");
    let empty = rows(execute_ok(
        "CALL algo.apsp('empty', 10) YIELD source_node, target_node, cost",
        &graph,
        &registry,
    ));
    assert_eq!(empty.row_count(), 0);
}

#[test]
fn algo_apsp_errors_with_too_large_detail_when_projection_exceeds_max_nodes() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(7_517, &["A", "B", "C"], &[]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let err = execute_result(
        "CALL algo.apsp('p', 2) YIELD source_node",
        &graph,
        &registry,
    )
    .expect_err("TooLarge maps to InvalidArgument");

    let detail = invalid_argument_detail(&err);
    assert!(detail.contains("max_nodes limit"));
    assert!(!detail.contains('3'));
    assert!(!detail.contains('2'));
}

#[test]
fn algo_apsp_rejects_negative_max_nodes_at_adapter() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(7_518, &["A"], &[]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let err = execute_result(
        "CALL algo.apsp('p', -1) YIELD source_node",
        &graph,
        &registry,
    )
    .expect_err("negative max_nodes rejected");

    assert!(invalid_argument_detail(&err).contains("non-negative"));
}

#[test]
fn algo_apsp_sorted_asc_by_source_then_target() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(
        7_519,
        &["A", "B", "C", "D"],
        &[(0, 1, 1.0), (1, 2, 1.0), (2, 3, 1.0), (0, 3, 5.0)],
    );
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "CALL algo.apsp('p', 10) YIELD source_node, target_node, cost",
        &graph,
        &registry,
    ));
    let pairs: Vec<_> = table
        .rows()
        .iter()
        .map(|row| {
            (
                node_ref(row.values(), 0).get(),
                node_ref(row.values(), 1).get(),
            )
        })
        .collect();

    assert!(pairs.windows(2).all(|window| window[0] < window[1]));
}

#[test]
fn algo_pathfinding_error_maps_negative_weight_to_invalid_argument_detail() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) =
        graph_with_labeled_weighted_edges(7_520, &["Source", "Target"], &[(0, 1, -7.25)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let err = execute_result(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD cost RETURN cost",
        &graph,
        &registry,
    )
    .expect_err("negative edge errors");

    assert_eq!(
        invalid_argument_detail(&err),
        "algo.dijkstra: traversed edge has negative weight"
    );
}

#[test]
fn algo_pathfinding_error_maps_nan_weight_to_invalid_argument_detail() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) =
        graph_with_labeled_weighted_edges(7_521, &["Source", "Target"], &[(0, 1, f64::NAN)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let err = execute_result(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD cost RETURN cost",
        &graph,
        &registry,
    )
    .expect_err("NaN edge errors");

    assert_eq!(
        invalid_argument_detail(&err),
        "algo.dijkstra: traversed edge has NaN weight"
    );
}

#[test]
fn algo_pathfinding_error_detail_does_not_leak_nodeids_or_weights() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) =
        graph_with_labeled_weighted_edges(7_522, &["Source", "Target"], &[(0, 1, -7.25)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let err = execute_result(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD cost RETURN cost",
        &graph,
        &registry,
    )
    .expect_err("negative edge errors");
    let detail = invalid_argument_detail(&err);

    assert!(detail.contains("weight"));
    assert!(!detail.contains("NodeId("));
    assert!(!detail.contains("-7.25"));
    for node in nodes {
        assert!(!detail.contains(&node.to_string()));
    }
}

#[test]
fn algo_sssp_default_weight_one_when_projection_has_no_weight_property() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) =
        graph_with_labeled_unweighted_edges(7_523, &["Source", "Mid", "Target"], &[(0, 1), (1, 2)]);
    build_unweighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source) CALL algo.sssp('p', a) YIELD target_node, cost RETURN target_node, cost",
        &graph,
        &registry,
    ));

    assert_eq!(
        table_values(&table),
        vec![
            vec![Value::NodeRef(nodes[0]), Value::Float(0.0)],
            vec![Value::NodeRef(nodes[1]), Value::Float(1.0)],
            vec![Value::NodeRef(nodes[2]), Value::Float(2.0)],
        ]
    );
}

#[test]
fn algo_dijkstra_accepts_node_ref_match_binding() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) =
        graph_with_labeled_weighted_edges(7_524, &["Source", "Target"], &[(0, 1, 1.0)]);
    build_weighted_projection(&graph, &registry, "p", &[]);

    let table = rows(execute_ok(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD path RETURN path",
        &graph,
        &registry,
    ));

    assert_eq!(
        table.rows()[0].values(),
        &[Value::List(vec![
            Value::NodeRef(nodes[0]),
            Value::NodeRef(nodes[1])
        ])]
    );
}

#[test]
fn algo_dijkstra_rejects_integer_arg_in_node_ref_slot_at_analyze_time() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    let err = analyze_failure("CALL algo.dijkstra('p', 1, 2) YIELD cost", &registry);
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
    assert_eq!(expected, ExpectedType::Specific(GqlType::NodeRef));
    assert_eq!(found, GqlType::Integer);
}

#[test]
fn value_extended_not_emitted_in_pathfinding_adapter_output() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_labeled_weighted_edges(
        7_525,
        &["Source", "Mid", "Target"],
        &[(0, 1, 1.0), (1, 2, 1.0)],
    );
    build_weighted_projection(&graph, &registry, "p", &[]);

    for source in [
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD cost, path, length RETURN cost, path, length",
        "MATCH (a:Source) CALL algo.sssp('p', a) YIELD target_node, cost RETURN target_node, cost",
        "CALL algo.apsp('p', 10) YIELD source_node, target_node, cost",
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
fn algo_pathfinding_adapters_two_graphs_same_name_no_collision() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph_a, _) =
        graph_with_labeled_weighted_edges(7_526, &["Source", "Target"], &[(0, 1, 1.0)]);
    let (graph_b, _) = graph_with_labeled_weighted_edges(
        7_527,
        &["Source", "Mid", "Target"],
        &[(0, 1, 1.0), (1, 2, 1.0)],
    );

    build_weighted_projection(&graph_a, &registry, "p", &[]);
    build_weighted_projection(&graph_b, &registry, "p", &[]);

    let rows_a = rows(execute_ok(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD length RETURN length",
        &graph_a,
        &registry,
    ));
    let rows_b = rows(execute_ok(
        "MATCH (a:Source), (b:Target) CALL algo.dijkstra('p', a, b) YIELD length RETURN length",
        &graph_b,
        &registry,
    ));

    assert_eq!(rows_a.rows()[0].values(), &[Value::Uint(2)]);
    assert_eq!(rows_b.rows()[0].values(), &[Value::Uint(3)]);
}

fn dijkstra_rows(result: Option<selene_algorithms::PathResult>) -> Vec<Vec<Value>> {
    let Some(result) = result else {
        return Vec::new();
    };
    let length = result.nodes.len() as u64;
    let path = result.nodes.into_iter().map(Value::NodeRef).collect();
    vec![vec![
        Value::Float(result.cost),
        Value::List(path),
        Value::Uint(length),
    ]]
}

fn sssp_rows(result: Vec<(NodeId, f64)>) -> Vec<Vec<Value>> {
    result
        .into_iter()
        .map(|(target_node, cost)| vec![Value::NodeRef(target_node), Value::Float(cost)])
        .collect()
}

fn apsp_rows(result: Vec<(NodeId, NodeId, f64)>) -> Vec<Vec<Value>> {
    result
        .into_iter()
        .map(|(source_node, target_node, cost)| {
            vec![
                Value::NodeRef(source_node),
                Value::NodeRef(target_node),
                Value::Float(cost),
            ]
        })
        .collect()
}

fn output_types(registry: &dyn ProcedureRegistry, name: &[&str]) -> Vec<GqlType> {
    let interned: Vec<_> = name.iter().map(|segment| istr(segment)).collect();
    registry
        .lookup(&interned)
        .unwrap_or_else(|| panic!("missing procedure {name:?}"))
        .output_schema
        .columns
        .into_iter()
        .map(|column| column.ty)
        .collect()
}

fn node_ids_in_column(table: &BindingTable, index: usize) -> Vec<NodeId> {
    table
        .rows()
        .iter()
        .map(|row| node_ref(row.values(), index))
        .collect()
}

fn node_ref(values: &[Value], index: usize) -> NodeId {
    match &values[index] {
        Value::NodeRef(node) => *node,
        other => panic!("expected NodeRef at column {index}, got {other:?}"),
    }
}

fn assert_no_extended(value: &Value) {
    match value {
        Value::List(values) => {
            for value in values {
                assert_no_extended(value);
            }
        }
        other => assert!(!matches!(other, Value::Extended { .. })),
    }
}
