//! Shared procedure-CALL plan-cache integration tests.

use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex, MutexGuard},
};

use selene_core::{GraphId, IStr, Value, intern};
use selene_gql::{
    CallPlanCache, GqlType, ProcedureContext, ProcedureError, ProcedureHandle, ProcedureMetadata,
    ProcedureMutability, ProcedureOutputColumn, ProcedureOutputSchema, ProcedureRegistry,
    ProcedureResult, ProcedureSignature, ProcedureTier, Session,
};
use selene_graph::{SharedGraph, TypedIndexKind};

#[derive(Debug)]
struct TestRegistry {
    name: Box<[IStr]>,
    metadata: ProcedureMetadata,
    records: Mutex<u64>,
}

impl TestRegistry {
    fn new() -> Self {
        Self {
            name: Box::from([istr("cache"), istr("values")]),
            metadata: ProcedureMetadata::new(
                ProcedureHandle::new(1),
                ProcedureSignature::new(Vec::new()),
                ProcedureOutputSchema {
                    columns: vec![ProcedureOutputColumn::new(istr("value"), GqlType::Integer)],
                },
                ProcedureTier::Graph,
                ProcedureMutability::Read,
                None,
            ),
            records: Mutex::new(0),
        }
    }

    fn records(&self) -> MutexGuard<'_, u64> {
        self.records.lock().expect("records mutex")
    }
}

impl ProcedureRegistry for TestRegistry {
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        (name == self.name.as_ref()).then(|| self.metadata.clone())
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        *self.records() += 1;
        Ok(ProcedureResult {
            rows: vec![vec![Value::Int(7)]],
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
    assert_eq!(stats.misses, 1);
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
        .create_property_index(istr("CachePerson"), istr("age"), TypedIndexKind::I64)
        .expect("schema change executes");
    session
        .execute_source("CALL cache.values() YIELD value", &registry)
        .expect("post-schema-change call executes");

    let stats = cache.stats();
    assert_eq!(stats.misses, 2);
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
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.hits, 1);
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

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn cache() -> Arc<CallPlanCache> {
    Arc::new(CallPlanCache::new(NonZeroUsize::new(8).expect("nonzero")))
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}
