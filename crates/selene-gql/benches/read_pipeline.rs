#![allow(missing_docs)]
//! Read-query pipeline execution bench: closes the read-execution coverage
//! gap (the declared 60%-read workload previously had only the correlated
//! subquery bench timing read execution).
//!
//! Warm-plan-cache rows run on a no-WAL in-memory `SharedGraph`, so the timed
//! body is pure execution + index access — not parse/plan/optimize and not
//! durability: label scan + indexed range filter, two-leg hash join, ORDER BY
//! top-K, high-cardinality GROUP BY, DISTINCT dedup, indexed `IN` bitmap union,
//! inline `CALL {}` table-subquery extension, single-item `LET`, composite
//! equality lookup, post-RETURN `LIMIT 10` (the B19 baseline), and pre-RETURN
//! `LIMIT 10` (the safe pattern cap row). Cold and shared-cache companions on
//! the cheapest row rebuild a fresh session per iteration to isolate
//! short-lived-session cache strategy.
//!
//! Fixture topology note: every `KNOWS` offset in `BenchFixture` is ≡1 mod 3,
//! so Person edges land on Sensor and Sensor edges land on Device —
//! Person→Person is ~empty. The join row targets Sensor/Device deliberately;
//! restricting the far legs to `:Person` would bench an empty join.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::{num::NonZeroUsize, sync::Arc};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{DbString, GraphId, LabelSet, PropertyMap, Value};
use selene_gql::{EmptyProcedureRegistry, Session, SharedPlanCache, StatementOutput};
use selene_graph::{SharedGraph, TypedIndexKind};
use selene_testing::BenchProfile;
use smallvec::smallvec;

/// Label scan + indexed `age` range filter + projection (~half of Persons).
const FILTER_PROJECT_Q: &str = "MATCH (n:Person) FILTER n.age >= 40 RETURN n.name AS name";
/// Two comma-separated legs sharing `s` lower to a hash join (build/probe).
const EXPAND_HASHJOIN_Q: &str = "MATCH (a:Person)-[:KNOWS]->(s:Sensor), \
     (s)-[:KNOWS]->(d:Device) RETURN a.name AS a_name, d.name AS d_name";
/// Full Person scan feeding a top-K sort; `score` is non-indexed.
const ORDER_BY_TOPK_Q: &str =
    "MATCH (n:Person) RETURN n.name AS name, n.score AS score ORDER BY n.score DESC LIMIT 10";
/// Hash-aggregate build over up to 1024 `score` groups.
const GROUP_BY_HIGHCARD_Q: &str =
    "MATCH (n:Person) RETURN n.score AS score, count(*) AS c GROUP BY n.score";
/// DISTINCT over 256 distinct `name` values (high dedup ratio).
const DISTINCT_DEDUP_Q: &str = "MATCH (n:Person) RETURN DISTINCT n.name AS name";
/// Small indexed `IN` list over the fixture's maintained `Person(name)` index.
const MATCH_NAME_IN_Q: &str = "MATCH (n:Person) FILTER n.name IN \
    ['bench-name-0', 'bench-name-3', 'bench-name-6', 'bench-name-9', \
     'bench-name-12', 'bench-name-15', 'bench-name-18', 'bench-name-21', \
     'bench-name-24', 'bench-name-27', 'bench-name-30', 'bench-name-33', \
     'bench-name-36', 'bench-name-39', 'bench-name-42', 'bench-name-45'] \
    RETURN n.name AS name";
/// Post-RETURN LIMIT with no ORDER BY — the B19 baseline remains scale-linear.
const MATCH_LIMIT10_Q: &str = "MATCH (n:Person) RETURN n.name AS name LIMIT 10";
/// Pre-RETURN LIMIT with no ORDER BY — can cap pattern materialization safely.
const MATCH_PRERETURN_LIMIT10_Q: &str = "MATCH (n:Person) LIMIT 10 RETURN n.name AS name";
/// Two-key equality lookup over a maintained `Person(age, name)` composite index.
const MATCH_COMPOSITE_LOOKUP_Q: &str =
    "MATCH (n:Person) FILTER n.age = 20 AND n.name = 'bench-name-0' RETURN n.name AS name";
/// Correlated inline `CALL {}` row extension with two yielded scalar columns.
const CALL_SUBQUERY_YIELD_Q: &str = "MATCH (a:Person) \
    CALL (a) { RETURN a.name AS name_copy, a.age AS age_copy LIMIT 1 } \
    YIELD name_copy, age_copy RETURN name_copy, age_copy";
/// Optional inline `CALL {}` null-yield row extension.
const OPTIONAL_CALL_SUBQUERY_NULL_YIELD_Q: &str = "MATCH (a:Person) \
    OPTIONAL CALL (a) { MATCH (a)-[:KNOWS]->(:Nope) RETURN 1 AS none } \
    YIELD none RETURN none";
/// Single-binding LET extension over the Person scan.
const LET_SINGLE_EXTEND_Q: &str = "MATCH (n:Person) LET doubled = n.age + n.age RETURN doubled";
/// Selective unanchored edge-property predicate used to A/B edge index access.
const EDGE_PROPERTY_FILTER_Q: &str =
    "MATCH ()-[e:CONNECTED_TO]->() WHERE e.from_port = 'port_17' RETURN e";

const WARM_ROWS: [(&str, &str); 11] = [
    ("match_filter_project", FILTER_PROJECT_Q),
    ("match_expand_hashjoin", EXPAND_HASHJOIN_Q),
    ("order_by_topk", ORDER_BY_TOPK_Q),
    ("group_by_highcard", GROUP_BY_HIGHCARD_Q),
    ("distinct_dedup", DISTINCT_DEDUP_Q),
    ("match_name_in", MATCH_NAME_IN_Q),
    ("call_subquery_yield", CALL_SUBQUERY_YIELD_Q),
    (
        "optional_call_subquery_null_yield",
        OPTIONAL_CALL_SUBQUERY_NULL_YIELD_Q,
    ),
    ("let_single_extend", LET_SINGLE_EXTEND_Q),
    ("match_limit10", MATCH_LIMIT10_Q),
    ("match_prereturn_limit10", MATCH_PRERETURN_LIMIT10_Q),
];

fn execute_read(session: &mut Session<'_>, source: &str) -> usize {
    match session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("read bench statement executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("read bench expects rows, got {other:?}"),
    }
}

fn bench_read_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_pipeline");
    for &scale in BenchProfile::from_env().scales() {
        // Fixture build happens once per scale, outside every timed routine.
        let state = common::gql_write_state_in_memory(scale);
        for (name, source) in WARM_ROWS {
            let mut session =
                Session::new(&state.graph).with_plan_cache(NonZeroUsize::new(64).expect("nonzero"));
            // Seat the plan cache so the timed body is a pure cache-hit
            // execute (no parse/analyze/plan/optimize). Also asserts the row
            // is non-degenerate where the fixture guarantees output.
            let primed = execute_read(&mut session, source);
            if matches!(
                name,
                "match_expand_hashjoin"
                    | "group_by_highcard"
                    | "distinct_dedup"
                    | "match_name_in"
                    | "call_subquery_yield"
                    | "optional_call_subquery_null_yield"
            ) {
                assert!(
                    primed > 0,
                    "{name} produced no rows — fixture topology mismatch"
                );
            }
            group.throughput(Throughput::Elements(scale as u64));
            group.bench_function(BenchmarkId::new(name, scale), |b| {
                b.iter(|| std::hint::black_box(execute_read(&mut session, source)));
            });
        }
        // Cold companion: a fresh cache-less session per iteration pays the
        // full parse/analyze/plan/optimize/execute pipeline. The graph is
        // shared (reads never mutate it); only the session is rebuilt.
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::new("match_limit10/cold", scale), |b| {
            b.iter_batched(
                || Session::new(&state.graph),
                |mut session| std::hint::black_box(execute_read(&mut session, MATCH_LIMIT10_Q)),
                BatchSize::SmallInput,
            );
        });
        let shared_cache = Arc::new(SharedPlanCache::new(
            NonZeroUsize::new(64).expect("nonzero"),
        ));
        let mut warmup =
            Session::new(&state.graph).with_shared_plan_cache(Arc::clone(&shared_cache));
        execute_read(&mut warmup, MATCH_LIMIT10_Q);
        assert_eq!(
            shared_cache.stats().hits,
            0,
            "warmup should miss then insert"
        );
        group.bench_function(BenchmarkId::new("match_limit10/shared_cache", scale), |b| {
            b.iter_batched(
                || Session::new(&state.graph).with_shared_plan_cache(Arc::clone(&shared_cache)),
                |mut session| std::hint::black_box(execute_read(&mut session, MATCH_LIMIT10_Q)),
                BatchSize::SmallInput,
            );
        });

        let graph = composite_index_graph(scale);
        let mut session =
            Session::new(&graph).with_plan_cache(NonZeroUsize::new(64).expect("nonzero"));
        let primed = execute_read(&mut session, MATCH_COMPOSITE_LOOKUP_Q);
        assert!(primed > 0, "match_composite_lookup produced no rows");
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::new("match_composite_lookup", scale), |b| {
            b.iter(|| std::hint::black_box(execute_read(&mut session, MATCH_COMPOSITE_LOOKUP_Q)));
        });

        for indexed in [false, true] {
            let graph = edge_property_graph(scale, indexed);
            let name = if indexed {
                "edge_property_filter_indexed"
            } else {
                "edge_property_filter_no_index"
            };
            let mut session =
                Session::new(&graph).with_plan_cache(NonZeroUsize::new(64).expect("nonzero"));
            let primed = execute_read(&mut session, EDGE_PROPERTY_FILTER_Q);
            assert!(primed > 0, "{name} produced no rows");
            group.throughput(Throughput::Elements(scale as u64));
            group.bench_function(BenchmarkId::new(name, scale), |b| {
                b.iter(|| std::hint::black_box(execute_read(&mut session, EDGE_PROPERTY_FILTER_Q)));
            });
        }
    }
    group.finish();
}

fn edge_property_graph(scale: usize, indexed: bool) -> SharedGraph {
    let scale = scale.max(1);
    let graph = SharedGraph::new(GraphId::new(if indexed { 1_101 } else { 1_100 }));
    let block = dbs("Block");
    let connected = dbs("CONNECTED_TO");
    let from_port = dbs("from_port");
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            let mut nodes = Vec::with_capacity(scale);
            for _ in 0..scale {
                nodes.push(
                    mutator
                        .create_node(LabelSet::single(block.clone()), PropertyMap::new())
                        .expect("edge bench node insert succeeds"),
                );
            }
            for idx in 0..scale {
                let source = nodes[idx];
                for offset in [1_usize, 7, 31] {
                    let target = nodes[(idx + offset) % scale];
                    let port = dbs(&format!("port_{}", (idx + offset) % 128));
                    mutator
                        .create_edge(
                            connected.clone(),
                            source,
                            target,
                            PropertyMap::from_pairs([(from_port.clone(), Value::String(port))])
                                .expect("edge bench properties fit"),
                        )
                        .expect("edge bench edge insert succeeds");
                }
            }
        }
        txn.commit().expect("edge bench fixture commits");
    }
    if indexed {
        graph
            .create_edge_property_index(connected, from_port, TypedIndexKind::String)
            .expect("edge property index builds");
    }
    graph
}

fn composite_index_graph(scale: usize) -> SharedGraph {
    let state = common::gql_write_state_in_memory(scale);
    let person = dbs("Person");
    let age = dbs("age");
    let name = dbs("name");
    {
        let mut txn = state.graph.begin_write();
        txn.mutator()
            .create_composite_property_index_named(
                person,
                smallvec![age, name],
                smallvec![TypedIndexKind::I64, TypedIndexKind::String],
                Some(dbs("idx_person_age_name")),
            )
            .expect("read bench composite index builds");
        txn.commit().expect("read bench composite index commits");
    }
    state.graph
}

fn dbs(value: &str) -> DbString {
    selene_core::db_string(value).expect("bench string fits DB string cap")
}

criterion_group! {
    name = read_pipeline;
    config = common::criterion_config();
    targets = bench_read_pipeline
}
criterion_main!(read_pipeline);
