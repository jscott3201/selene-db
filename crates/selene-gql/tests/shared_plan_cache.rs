//! Shared source-plan cache integration tests.

use std::{num::NonZeroUsize, sync::Arc};

use selene_core::{DbString, GraphId, Value};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, ImplDefinedCaps, Session, SharedPlanCache,
    StatementOutput,
};
use selene_graph::{SharedGraph, TypedIndexKind};

#[test]
fn shared_plan_cache_hits_across_short_lived_read_sessions() {
    let graph = graph(12_301);
    let cache = cache();
    let source = "RETURN 1 AS value";

    let first = Session::new(&graph)
        .with_shared_plan_cache(Arc::clone(&cache))
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("first read executes");
    let second = Session::new(&graph)
        .with_shared_plan_cache(Arc::clone(&cache))
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("second read executes");

    assert_eq!(single_int(first), 1);
    assert_eq!(single_int(second), 1);
    let stats = cache.stats();
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
}

#[test]
fn shared_plan_cache_caches_parameterized_writes() {
    let graph = graph(12_302);
    let cache = cache();
    let source = "INSERT (n:Sensor {id: $id}) RETURN n.id AS id";

    let mut first = Session::new(&graph).with_shared_plan_cache(Arc::clone(&cache));
    first.bind_parameter(db_string("id"), Value::Int(1));
    let first = first
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("first write executes");

    let mut second = Session::new(&graph).with_shared_plan_cache(Arc::clone(&cache));
    second.bind_parameter(db_string("id"), Value::Int(2));
    let second = second
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("second write executes");

    assert_eq!(single_int(first), 1);
    assert_eq!(single_int(second), 2);
    let stats = cache.stats();
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
}

#[test]
fn shared_plan_cache_schema_version_change_misses_next_lookup() {
    let graph = graph(12_303);
    let cache = cache();
    let source = "RETURN 1 AS value";

    Session::new(&graph)
        .with_shared_plan_cache(Arc::clone(&cache))
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("first read executes");
    graph
        .create_property_index(
            db_string("CachePerson"),
            db_string("age"),
            TypedIndexKind::I64,
        )
        .expect("schema change executes");
    Session::new(&graph)
        .with_shared_plan_cache(Arc::clone(&cache))
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("post-DDL read executes");

    let stats = cache.stats();
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.hits, 0);
}

#[test]
fn shared_plan_cache_separates_parameter_type_shape() {
    let graph = graph(12_304);
    let cache = cache();

    let mut untyped = Session::new(&graph).with_shared_plan_cache(Arc::clone(&cache));
    untyped.bind_parameter(db_string("id"), Value::Int(7));
    assert_eq!(
        single_int(
            untyped
                .execute_source("RETURN $id AS id", &EmptyProcedureRegistry)
                .expect("untyped parameter read executes"),
        ),
        7,
    );

    let mut typed = Session::new(&graph).with_shared_plan_cache(Arc::clone(&cache));
    typed.bind_parameter(db_string("id"), Value::Int(7));
    assert_eq!(
        single_int(
            typed
                .execute_source("RETURN $id :: INTEGER AS id", &EmptyProcedureRegistry)
                .expect("typed parameter read executes"),
        ),
        7,
    );

    let stats = cache.stats();
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.hits, 0);
}

#[test]
fn shared_plan_cache_rechecks_typed_parameter_values_on_hit() {
    let graph = graph(12_306);
    let cache = cache();
    let source = "RETURN $id :: INTEGER AS id";

    let mut first = Session::new(&graph).with_shared_plan_cache(Arc::clone(&cache));
    first.bind_parameter(db_string("id"), Value::Int(7));
    assert_eq!(
        single_int(
            first
                .execute_source(source, &EmptyProcedureRegistry)
                .expect("first typed parameter read executes"),
        ),
        7,
    );

    let mut second = Session::new(&graph).with_shared_plan_cache(Arc::clone(&cache));
    second.bind_parameter(db_string("id"), Value::String(db_string("wrong")));
    let err = second
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("cached plan still validates runtime parameter type");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "STRING",
            ..
        } if name.as_str() == "id" && expected == "INTEGER"
    ));

    let stats = cache.stats();
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
}

#[test]
fn shared_plan_cache_separates_impl_defined_caps() {
    let graph = graph(12_305);
    let cache = cache();
    let source = "RETURN 1 AS value";

    Session::new(&graph)
        .with_shared_plan_cache(Arc::clone(&cache))
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("default-cap read executes");
    Session::new(&graph)
        .with_impl_defined_caps(ImplDefinedCaps::DEFAULT.with_max_list_length(1))
        .with_shared_plan_cache(Arc::clone(&cache))
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("alternate-cap read executes");

    let stats = cache.stats();
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.hits, 0);
}

fn single_int(output: StatementOutput) -> i64 {
    let table = match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("write returned rows"),
        other => panic!("expected rows, got {other:?}"),
    };
    assert_eq!(table.row_count(), 1);
    let row = table.rows().first().expect("one row");
    let Value::Int(value) = row.values().first().expect("one value") else {
        panic!("expected integer");
    };
    *value
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn cache() -> Arc<SharedPlanCache> {
    Arc::new(SharedPlanCache::new(NonZeroUsize::new(8).expect("nonzero")))
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}
