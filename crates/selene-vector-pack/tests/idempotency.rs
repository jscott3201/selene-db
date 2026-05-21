//! Idempotency contract tests for vector index lifecycle procedures.

use std::sync::Arc;

use selene_core::{GraphId, Value};
use selene_gql::{ExecutorError, GqlStatus, Session};
use selene_graph::{IndexProvider, SharedGraph};
use selene_vector::{HnswConfig, HnswIndexRegistry, IvfConfig, IvfIndexRegistry};
use selene_vector_pack::VectorPack;

fn graph_with_vector_registries(id: u64) -> SharedGraph {
    let hnsw = Arc::new(
        HnswIndexRegistry::new(HnswConfig::new(2).expect("HNSW config builds"))
            .expect("HNSW registry builds"),
    );
    let ivf = Arc::new(
        IvfIndexRegistry::new(IvfConfig::new(2).expect("IVF config builds"))
            .expect("IVF registry builds"),
    );
    SharedGraph::builder(GraphId::new(id))
        .with_provider(hnsw as Arc<dyn IndexProvider>)
        .with_provider(ivf as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds")
}

fn execute(session: &mut Session<'_>, source: &str) -> Result<(), ExecutorError> {
    let pack = VectorPack::new();
    let registry = pack
        .registry_with_builtins()
        .expect("vector pack registers cleanly");
    session.execute_source(source, &registry).map(|_| ())
}

#[test]
fn vector_create_index_matching_config_is_idempotent() {
    let graph = graph_with_vector_registries(128_001);
    let mut session = Session::new(&graph);

    execute(
        &mut session,
        "CALL vector.create_index('episodes', 'hnsw', {dim: 2})",
    )
    .expect("fresh index creates");
    execute(
        &mut session,
        "CALL vector.create_index('episodes', 'hnsw', {dim: 2})",
    )
    .expect("matching config is a no-op");

    let names = list_index_names(&mut session);
    assert_eq!(
        names
            .iter()
            .filter(|name| name.as_str() == "episodes")
            .count(),
        1
    );
}

#[test]
fn vector_create_index_rejects_mismatched_config() {
    let graph = graph_with_vector_registries(128_002);
    let mut session = Session::new(&graph);

    execute(
        &mut session,
        "CALL vector.create_index('episodes', 'hnsw', {dim: 2})",
    )
    .expect("fresh index creates");
    let err = execute(
        &mut session,
        "CALL vector.create_index('episodes', 'hnsw', {dim: 3})",
    )
    .expect_err("mismatched config errors");

    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_PROCEDURE_ARGUMENT);
}

#[test]
fn vector_create_index_fresh_name_creates() {
    let graph = graph_with_vector_registries(128_003);
    let mut session = Session::new(&graph);

    execute(
        &mut session,
        "CALL vector.create_index('fresh', 'hnsw', {dim: 2})",
    )
    .expect("fresh index creates");

    assert!(
        list_index_names(&mut session)
            .iter()
            .any(|name| name == "fresh")
    );
}

fn list_index_names(session: &mut Session<'_>) -> Vec<String> {
    let pack = VectorPack::new();
    let registry = pack
        .registry_with_builtins()
        .expect("vector pack registers cleanly");
    let output = session
        .execute_source("CALL vector.list_indexes() YIELD name", &registry)
        .expect("list indexes executes");
    let table = match output {
        selene_gql::StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    };
    table
        .rows()
        .iter()
        .map(|row| match &row.values()[0] {
            Value::String(value) => value.as_str().to_owned(),
            Value::ExternalString(value) => value.as_ref().to_owned(),
            other => panic!("expected string name, got {other:?}"),
        })
        .collect()
}
