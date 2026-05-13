//! Projection procedure adapter tests.

mod common;

use common::{execute_ok, execute_result, graph_with_edges, istr, registry, rows, value_string};
use selene_algorithms_pack::AlgorithmsPack;
use selene_core::{LabelSet, PropertyMap, Value};
use selene_gql::{ExecutorError, ProcedureError};

#[test]
fn algo_projection_build_creates_named_projection_in_catalog() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_201, &[(0, 1), (1, 2)]);

    execute_ok(
        "CALL algo.projection_build('p', ['Person'], ['LINK'], NULL)",
        &graph,
        &registry,
    );
    let table = rows(execute_ok(
        "CALL algo.projection_get('p') YIELD name, generation, node_count, edge_count",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 1);
    let values = table.rows()[0].values();
    assert_eq!(value_string(&values[0]), "p");
    assert_eq!(values[1], Value::Uint(graph.read().meta.generation));
    assert_eq!(values[2], Value::Uint(3));
    assert_eq!(values[3], Value::Uint(2));
}

#[test]
fn algo_projection_get_returns_metadata_for_existing_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_202, &[(0, 1)]);
    common::build_projection(&graph, &registry, "p");

    let table = rows(execute_ok(
        "CALL algo.projection_get('p') YIELD name, node_count, edge_count",
        &graph,
        &registry,
    ));

    assert_eq!(value_string(&table.rows()[0].values()[0]), "p");
    assert_eq!(table.rows()[0].values()[1], Value::Uint(2));
    assert_eq!(table.rows()[0].values()[2], Value::Uint(1));
}

#[test]
fn algo_projection_get_errors_on_missing_name() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_203, &[(0, 1)]);

    let err = execute_result(
        "CALL algo.projection_get('missing') YIELD name",
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
fn algo_projection_list_enumerates_all_live_projections() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_204, &[(0, 1)]);
    common::build_projection(&graph, &registry, "b");
    common::build_projection(&graph, &registry, "a");

    let table = rows(execute_ok(
        "CALL algo.projection_list() YIELD name, node_count",
        &graph,
        &registry,
    ));

    let names: Vec<_> = table
        .rows()
        .iter()
        .map(|row| value_string(&row.values()[0]).to_owned())
        .collect();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(table.rows()[0].values()[1], Value::Uint(2));
    assert_eq!(table.rows()[1].values()[1], Value::Uint(2));
}

#[test]
fn algo_projection_drop_removes_named_projection() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_205, &[(0, 1)]);
    common::build_projection(&graph, &registry, "p");

    execute_ok("CALL algo.projection_drop('p')", &graph, &registry);
    let table = rows(execute_ok(
        "CALL algo.projection_list() YIELD name",
        &graph,
        &registry,
    ));

    assert_eq!(table.row_count(), 0);
}

#[test]
fn algo_projection_drop_idempotent_on_missing_name() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_206, &[(0, 1)]);

    execute_ok("CALL algo.projection_drop('missing')", &graph, &registry);
    execute_ok("CALL algo.projection_drop('missing')", &graph, &registry);
}

#[test]
fn projection_catalog_rebuilds_on_snapshot_generation_mismatch() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_207, &[(0, 1)]);
    common::build_projection(&graph, &registry, "p");

    let mut txn = graph.begin_write();
    txn.mutator()
        .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
        .expect("new node inserts");
    txn.commit().expect("mutation commits");

    let table = rows(execute_ok(
        "CALL algo.projection_get('p') YIELD generation, node_count",
        &graph,
        &registry,
    ));

    assert_eq!(
        table.rows()[0].values()[0],
        Value::Uint(graph.read().meta.generation)
    );
    assert_eq!(table.rows()[0].values()[1], Value::Uint(3));
}

#[test]
fn projection_catalog_survives_transaction_boundary_within_engine() {
    let pack = AlgorithmsPack::new();
    let registry = registry(&pack);
    let (graph, _) = graph_with_edges(7_208, &[(0, 1)]);

    common::build_projection(&graph, &registry, "p");
    let table = rows(execute_ok(
        "CALL algo.projection_get('p') YIELD node_count",
        &graph,
        &registry,
    ));

    assert_eq!(table.rows()[0].values()[0], Value::Uint(2));
}
