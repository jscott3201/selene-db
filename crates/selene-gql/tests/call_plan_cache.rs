//! Shared procedure-CALL plan-cache integration tests.

use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex, MutexGuard},
};

use selene_core::{DbString, GraphId, Value};
use selene_gql::{
    CallPlanCache, GqlType, ProcedureContext, ProcedureError, ProcedureHandle, ProcedureMetadata,
    ProcedureMutability, ProcedureOutputColumn, ProcedureOutputSchema, ProcedureRegistry,
    ProcedureResult, ProcedureSignature, ProcedureTier, Session,
};
use selene_graph::{SharedGraph, TypedIndexKind};

#[derive(Debug)]
struct TestRegistry {
    name: Box<[DbString]>,
    metadata: ProcedureMetadata,
    version: u64,
    value: i64,
    records: Mutex<u64>,
}

impl TestRegistry {
    fn new() -> Self {
        Self::with_version_handle_and_value(1, ProcedureHandle::new(1), 7)
    }

    fn with_version_handle_and_value(version: u64, handle: ProcedureHandle, value: i64) -> Self {
        Self {
            name: Box::from([db_string("cache"), db_string("values")]),
            metadata: ProcedureMetadata::new(
                handle,
                ProcedureSignature::new(Vec::new()),
                ProcedureOutputSchema {
                    columns: vec![ProcedureOutputColumn::new(
                        db_string("value"),
                        GqlType::Integer,
                    )],
                },
                ProcedureTier::Graph,
                ProcedureMutability::Read,
            ),
            version,
            value,
            records: Mutex::new(0),
        }
    }

    fn records(&self) -> MutexGuard<'_, u64> {
        self.records.lock().expect("records mutex")
    }
}

impl ProcedureRegistry for TestRegistry {
    fn lookup(&self, name: &[DbString]) -> Option<ProcedureMetadata> {
        (name == self.name.as_ref()).then(|| self.metadata.clone())
    }

    fn registry_version(&self) -> u64 {
        self.version
    }

    fn execute(
        &self,
        handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        if handle != self.metadata.handle {
            return Err(ProcedureError::UnknownProcedure {
                name: self.name.clone(),
            });
        }
        *self.records() += 1;
        Ok(ProcedureResult {
            rows: vec![vec![Value::Int(self.value)]],
        })
    }
}

#[test]
fn call_plan_cache_miss_then_hit_across_short_lived_sessions() {
    let registry = TestRegistry::new();
    let graph = graph(12_201);
    let cache = cache();

    Session::new(&graph)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source("CALL cache.values() YIELD value", &registry)
        .expect("first call executes");
    Session::new(&graph)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source("CALL cache.values() YIELD value", &registry)
        .expect("second call executes");

    let stats = cache.stats();
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.hits, 1);
    assert_eq!(*registry.records(), 2);
}

#[test]
fn call_plan_cache_schema_version_change_misses_next_lookup() {
    let registry = TestRegistry::new();
    let graph = graph(12_202);
    let cache = cache();
    let mut session = Session::new(&graph).with_call_plan_cache(Arc::clone(&cache));

    session
        .execute_source("CALL cache.values() YIELD value", &registry)
        .expect("first call executes");
    session
        .execute_source("CALL cache.values() YIELD value", &registry)
        .expect("second call hits cache");
    graph
        .create_property_index(
            db_string("CachePerson"),
            db_string("age"),
            TypedIndexKind::I64,
        )
        .expect("schema change executes");
    session
        .execute_source("CALL cache.values() YIELD value", &registry)
        .expect("post-schema-change call executes");

    let stats = cache.stats();
    assert_eq!(stats.misses, 4);
    assert_eq!(stats.hits, 1);
}

#[test]
fn call_plan_cache_graph_id_separates_shared_cache_keys() {
    let registry = TestRegistry::new();
    let cache = cache();
    let graph_one = graph(12_203);
    let graph_two = graph(12_204);

    Session::new(&graph_one)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source("CALL cache.values() YIELD value", &registry)
        .expect("first graph executes");
    Session::new(&graph_two)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source("CALL cache.values() YIELD value", &registry)
        .expect("second graph executes");
    Session::new(&graph_one)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source("CALL cache.values() YIELD value", &registry)
        .expect("first graph second call executes");

    let stats = cache.stats();
    assert_eq!(stats.misses, 4);
    assert_eq!(stats.hits, 1);
}

#[test]
fn call_plan_cache_registry_version_separates_shared_cache_keys() {
    let registry_one = TestRegistry::with_version_handle_and_value(1, ProcedureHandle::new(1), 7);
    let registry_two = TestRegistry::with_version_handle_and_value(2, ProcedureHandle::new(2), 11);
    let graph = graph(12_206);
    let cache = cache();

    let first = Session::new(&graph)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source("CALL cache.values() YIELD value", &registry_one)
        .expect("first registry executes");
    let second = Session::new(&graph)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source("CALL cache.values() YIELD value", &registry_two)
        .expect("second registry executes");
    let third = Session::new(&graph)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source("CALL cache.values() YIELD value", &registry_two)
        .expect("second registry repeats");

    assert_eq!(single_int(first), 7);
    assert_eq!(single_int(second), 11);
    assert_eq!(single_int(third), 11);
    assert_eq!(*registry_one.records(), 1);
    assert_eq!(*registry_two.records(), 2);
    assert_eq!(cache.stats().hits, 1);
}

#[test]
fn embedded_pipeline_call_does_not_touch_call_plan_cache() {
    let registry = TestRegistry::new();
    let graph = graph(12_205);
    let cache = cache();
    let mut session = Session::new(&graph).with_call_plan_cache(Arc::clone(&cache));

    session
        .execute_source("INSERT (:Probe)", &registry)
        .expect("seed node");
    session
        .execute_source(
            "MATCH (n) CALL cache.values() YIELD value RETURN value",
            &registry,
        )
        .expect("first embedded call executes");
    session
        .execute_source(
            "MATCH (n) CALL cache.values() YIELD value RETURN value",
            &registry,
        )
        .expect("second embedded call executes");

    assert_eq!(cache.stats(), Default::default());
}

fn single_int(output: selene_gql::StatementOutput) -> i64 {
    let selene_gql::StatementOutput::Rows(table) = output else {
        panic!("expected rows");
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

fn cache() -> Arc<CallPlanCache> {
    Arc::new(CallPlanCache::new(NonZeroUsize::new(8).expect("nonzero")))
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}
