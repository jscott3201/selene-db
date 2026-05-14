//! Structural procedure adapter tests.

mod common;

use std::collections::HashSet;

use common::{
    analyze_err, build_projection, execute_ok, execute_result, graph_with_edges, istr, registry,
    rows,
};
use selene_algorithms::{
    GraphProjection, ProjectionCatalog, ProjectionConfig, articulation_points, bridges, scc,
    topological_sort, wcc,
};
use selene_algorithms_pack::AlgorithmsPack;
use selene_core::Value;
use selene_gql::{ExecutorError, GqlType, ProcedureError, ProcedureRegistry};
use selene_graph::SharedGraph;

#[test]
fn algo_wcc_returns_node_component_rows_matching_direct_api_call() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_401, &[(0, 1), (2, 3)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.wcc('p') YIELD node_id, component_id",
        &graph,
        &registry,
    ));
    let expected = direct_rows(&graph, "p", |projection| {
        wcc(projection)
            .into_iter()
            .map(node_component_row)
            .collect()
    });

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_wcc_errors_on_missing_projection_name() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_402, &[(0, 1)]);

    assert_invalid_argument(
        execute_result("CALL algo.wcc('missing') YIELD node_id", &graph, &registry)
            .expect_err("missing projection errors"),
    );
}

#[test]
fn algo_wcc_component_ids_are_value_uint_u64() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_403, &[(0, 1)]);
    build_projection(&graph, &registry, "p");

    assert_eq!(
        output_types(&registry, &["algo", "wcc"]),
        vec![GqlType::NodeRef, GqlType::Uint64]
    );
    let table = rows(execute_ok(
        "CALL algo.wcc('p') YIELD node_id, component_id",
        &graph,
        &registry,
    ));

    assert!(matches!(table.rows()[0].values()[1], Value::Uint(_)));
}

#[test]
fn algo_scc_returns_node_component_rows_matching_direct_api_call() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_404, &[(0, 1), (1, 0), (2, 3)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.scc('p') YIELD node_id, component_id",
        &graph,
        &registry,
    ));
    let expected = direct_rows(&graph, "p", |projection| {
        scc(projection)
            .into_iter()
            .map(node_component_row)
            .collect()
    });

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_scc_errors_on_missing_projection_name() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_405, &[(0, 1)]);

    assert_invalid_argument(
        execute_result("CALL algo.scc('missing') YIELD node_id", &graph, &registry)
            .expect_err("missing projection errors"),
    );
}

#[test]
fn algo_scc_component_ids_are_value_uint_u64() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_406, &[(0, 1), (1, 0)]);
    build_projection(&graph, &registry, "p");

    assert_eq!(
        output_types(&registry, &["algo", "scc"]),
        vec![GqlType::NodeRef, GqlType::Uint64]
    );
    let table = rows(execute_ok(
        "CALL algo.scc('p') YIELD node_id, component_id",
        &graph,
        &registry,
    ));

    assert!(matches!(table.rows()[0].values()[1], Value::Uint(_)));
}

#[test]
fn algo_topological_sort_returns_node_position_rows_in_topological_order() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_407, &[(0, 1), (0, 2), (1, 3), (2, 3)]);
    build_projection(&graph, &registry, "p");

    assert_eq!(
        output_types(&registry, &["algo", "topological_sort"]),
        vec![GqlType::NodeRef, GqlType::Uint64]
    );
    assert_eq!(
        output_names(&registry, &["algo", "topological_sort"]),
        vec!["node_id".to_string(), "topo_position".to_string()]
    );
    let table = rows(execute_ok(
        "CALL algo.topological_sort('p') YIELD node_id, topo_position",
        &graph,
        &registry,
    ));
    let expected = direct_rows(&graph, "p", |projection| {
        topological_sort(projection)
            .expect("fixture is a DAG")
            .into_iter()
            .map(|(node, position)| vec![Value::NodeRef(node), Value::Uint(position as u64)])
            .collect()
    });

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_topological_sort_errors_on_cyclic_projection_with_invalid_argument_shape() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_408, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let err = execute_result(
        "CALL algo.topological_sort('p') YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("cycle errors");

    let detail = invalid_argument_detail(&err);
    assert!(detail.contains("cycle"));
}

#[test]
fn algo_topological_sort_empty_projection_returns_no_rows() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_409, &[(0, 1)]);
    build_empty_projection(&graph, &registry, "empty");

    let table = rows(execute_ok(
        "CALL algo.topological_sort('empty') YIELD node_id, topo_position",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_articulation_points_returns_node_rows_matching_direct_api_call() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_410, &[(0, 1), (1, 2), (1, 3)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.articulation_points('p') YIELD node_id",
        &graph,
        &registry,
    ));
    let expected = direct_rows(&graph, "p", |projection| {
        articulation_points(projection)
            .into_iter()
            .map(|node| vec![Value::NodeRef(node)])
            .collect()
    });

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_articulation_points_empty_projection_returns_no_rows() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_411, &[(0, 1)]);
    build_empty_projection(&graph, &registry, "empty");

    let table = rows(execute_ok(
        "CALL algo.articulation_points('empty') YIELD node_id",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_bridges_returns_edge_pair_rows_matching_direct_api_call() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_412, &[(0, 1), (1, 2), (3, 4), (4, 5), (5, 3)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.bridges('p') YIELD from_node, to_node",
        &graph,
        &registry,
    ));
    let expected = direct_rows(&graph, "p", |projection| {
        bridges(projection)
            .into_iter()
            .map(|(from, to)| vec![Value::NodeRef(from), Value::NodeRef(to)])
            .collect()
    });

    assert_eq!(table_values(&table), expected);
}

#[test]
fn algo_bridges_emits_canonical_lower_id_first_endpoints() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_edges(7_413, &[(1, 0)]);
    build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.bridges('p') YIELD from_node, to_node",
        &graph,
        &registry,
    ));

    assert_eq!(
        table.rows()[0].values(),
        &[Value::NodeRef(nodes[0]), Value::NodeRef(nodes[1])]
    );
}

#[test]
fn algo_wcc_count_returns_single_uint_count_row() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_414, &[(0, 1), (2, 3)]);
    build_projection(&graph, &registry, "p");

    assert_eq!(
        output_types(&registry, &["algo", "wcc_count"]),
        vec![GqlType::Uint64]
    );
    let table = rows(execute_ok(
        "CALL algo.wcc_count('p') YIELD count",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 1);
    assert_eq!(table.rows()[0].values(), &[Value::Uint(2)]);
}

#[test]
fn algo_scc_count_returns_single_uint_count_row() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_415, &[(0, 1), (1, 0), (1, 2)]);
    build_projection(&graph, &registry, "p");

    assert_eq!(
        output_types(&registry, &["algo", "scc_count"]),
        vec![GqlType::Uint64]
    );
    let table = rows(execute_ok(
        "CALL algo.scc_count('p') YIELD count",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 1);
    assert_eq!(table.rows()[0].values(), &[Value::Uint(2)]);
}

#[test]
fn algo_count_procedures_return_zero_on_empty_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_416, &[(0, 1)]);
    build_empty_projection(&graph, &registry, "empty");

    let wcc_rows = rows(execute_ok(
        "CALL algo.wcc_count('empty') YIELD count",
        &graph,
        &registry,
    ));
    let scc_rows = rows(execute_ok(
        "CALL algo.scc_count('empty') YIELD count",
        &graph,
        &registry,
    ));

    assert_eq!(wcc_rows.rows()[0].values(), &[Value::Uint(0)]);
    assert_eq!(scc_rows.rows()[0].values(), &[Value::Uint(0)]);
}

#[test]
fn algo_wcc_count_matches_unique_component_ids_in_wcc_on_nonempty_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_417, &[(0, 1), (2, 3), (4, 4)]);
    build_projection(&graph, &registry, "p");

    let wcc_rows = rows(execute_ok(
        "CALL algo.wcc('p') YIELD node_id, component_id",
        &graph,
        &registry,
    ));
    let count_rows = rows(execute_ok(
        "CALL algo.wcc_count('p') YIELD count",
        &graph,
        &registry,
    ));
    let unique_components: HashSet<_> = wcc_rows
        .rows()
        .iter()
        .map(|row| component_id(row.values()))
        .collect();

    assert_eq!(unique_components.len(), 3);
    assert_eq!(
        count_rows.rows()[0].values(),
        &[Value::Uint(unique_components.len() as u64)]
    );
}

#[test]
fn algo_scc_count_matches_unique_component_ids_in_scc_on_nonempty_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_418, &[(0, 1), (1, 0), (2, 3), (3, 2), (3, 4)]);
    build_projection(&graph, &registry, "p");

    let scc_rows = rows(execute_ok(
        "CALL algo.scc('p') YIELD node_id, component_id",
        &graph,
        &registry,
    ));
    let count_rows = rows(execute_ok(
        "CALL algo.scc_count('p') YIELD count",
        &graph,
        &registry,
    ));
    let unique_components: HashSet<_> = scc_rows
        .rows()
        .iter()
        .map(|row| component_id(row.values()))
        .collect();

    assert_eq!(unique_components.len(), 3);
    assert_eq!(
        count_rows.rows()[0].values(),
        &[Value::Uint(unique_components.len() as u64)]
    );
}

#[test]
fn algo_wcc_two_graphs_same_name_no_collision() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph_a, _) = graph_with_edges(7_419, &[(0, 1)]);
    let (graph_b, _) = graph_with_edges(7_420, &[(0, 1), (2, 3)]);

    build_projection(&graph_a, &registry, "p");
    build_projection(&graph_b, &registry, "p");

    let rows_a = rows(execute_ok(
        "CALL algo.wcc('p') YIELD node_id, component_id",
        &graph_a,
        &registry,
    ));
    let rows_b = rows(execute_ok(
        "CALL algo.wcc('p') YIELD node_id, component_id",
        &graph_b,
        &registry,
    ));

    assert_eq!(rows_a.row_count(), 2);
    assert_eq!(rows_b.row_count(), 4);
}

#[test]
fn value_extended_not_emitted_in_structural_adapter_output() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_421, &[(0, 1), (1, 2)]);
    build_projection(&graph, &registry, "p");

    for source in [
        "CALL algo.wcc('p') YIELD node_id, component_id",
        "CALL algo.scc('p') YIELD node_id, component_id",
        "CALL algo.topological_sort('p') YIELD node_id, topo_position",
        "CALL algo.articulation_points('p') YIELD node_id",
        "CALL algo.bridges('p') YIELD from_node, to_node",
        "CALL algo.wcc_count('p') YIELD count",
        "CALL algo.scc_count('p') YIELD count",
    ] {
        let table = rows(execute_ok(source, &graph, &registry));
        for row in table.rows() {
            for value in row.values() {
                assert!(!matches!(value, Value::Extended { .. }));
            }
        }
    }
}

#[test]
fn algo_structural_adapters_all_require_exact_arity_one() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);

    for procedure in [
        "wcc",
        "scc",
        "wcc_count",
        "scc_count",
        "topological_sort",
        "articulation_points",
        "bridges",
    ] {
        let no_args = analyze_err(&format!("CALL algo.{procedure}() YIELD *"), &registry);
        let two_args = analyze_err(
            &format!("CALL algo.{procedure}('p', 'extra') YIELD *"),
            &registry,
        );

        assert!(no_args.contains("argument"));
        assert!(two_args.contains("argument"));
    }
}

#[test]
fn algo_topological_sort_cycle_hint_omitted_from_detail() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, nodes) = graph_with_edges(7_422, &[(0, 1), (1, 2), (2, 0)]);
    build_projection(&graph, &registry, "p");

    let err = execute_result(
        "CALL algo.topological_sort('p') YIELD node_id",
        &graph,
        &registry,
    )
    .expect_err("cycle errors");

    let detail = invalid_argument_detail(&err);
    assert!(detail.contains("cycle"));
    assert!(!detail.contains("NodeId("));
    for node in nodes {
        assert!(!detail.contains(&node.to_string()));
    }
}

fn build_empty_projection(graph: &SharedGraph, registry: &dyn ProcedureRegistry, name: &str) {
    execute_ok(
        &format!("CALL algo.projection_build('{name}', ['MissingLabel'], NULL, NULL)"),
        graph,
        registry,
    );
}

fn direct_rows(
    graph: &SharedGraph,
    name: &str,
    f: impl FnOnce(&GraphProjection) -> Vec<Vec<Value>>,
) -> Vec<Vec<Value>> {
    let snapshot = graph.read();
    let catalog = ProjectionCatalog::new();
    catalog
        .project(
            &snapshot,
            &ProjectionConfig {
                name: name.to_string(),
                node_labels: Vec::new(),
                edge_labels: Vec::new(),
                weight_property: None,
            },
            None,
        )
        .expect("projection builds");
    let projection = catalog.get(name).expect("projection exists");
    f(projection.projection())
}

fn node_component_row((node, component_id): (selene_core::NodeId, u64)) -> Vec<Value> {
    vec![Value::NodeRef(node), Value::Uint(component_id)]
}

fn component_id(values: &[Value]) -> u64 {
    match &values[1] {
        Value::Uint(component_id) => *component_id,
        other => panic!("component_id must be Value::Uint, got {other:?}"),
    }
}

fn table_values(table: &selene_gql::BindingTable) -> Vec<Vec<Value>> {
    table
        .rows()
        .iter()
        .map(|row| row.values().to_vec())
        .collect()
}

fn output_types(registry: &dyn ProcedureRegistry, name: &[&str]) -> Vec<GqlType> {
    metadata_columns(registry, name)
        .into_iter()
        .map(|column| column.ty)
        .collect()
}

fn output_names(registry: &dyn ProcedureRegistry, name: &[&str]) -> Vec<String> {
    metadata_columns(registry, name)
        .into_iter()
        .map(|column| column.name.as_str().to_owned())
        .collect()
}

fn metadata_columns(
    registry: &dyn ProcedureRegistry,
    name: &[&str],
) -> Vec<selene_gql::ProcedureOutputColumn> {
    let interned: Vec<_> = name.iter().map(|segment| istr(segment)).collect();
    registry
        .lookup(&interned)
        .unwrap_or_else(|| panic!("missing procedure {name:?}"))
        .output_schema
        .columns
}

fn assert_invalid_argument(err: ExecutorError) {
    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

fn invalid_argument_detail(err: &ExecutorError) -> &str {
    let ExecutorError::Procedure {
        source: ProcedureError::InvalidArgument { detail },
        ..
    } = err
    else {
        panic!("expected InvalidArgument procedure error, got {err:?}");
    };
    detail
}
